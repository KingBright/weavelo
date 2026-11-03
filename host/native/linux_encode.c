/* FFmpeg's existing VAAPI path, in process: no raw-frame pipe or AU lookahead.
 * Only upload/VPP/codec work runs here; ownership, pacing and sessions stay Rust. */
#include <libavcodec/avcodec.h>
#include <libavcodec/bsf.h>
#include <libavfilter/avfilter.h>
#include <libavfilter/buffersink.h>
#include <libavfilter/buffersrc.h>
#include <libavutil/hwcontext.h>
#include <libavutil/opt.h>
#include <libavutil/pixdesc.h>
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <time.h>

#if LIBAVCODEC_VERSION_MAJOR != 62 || LIBAVUTIL_VERSION_MAJOR != 60 || LIBAVFILTER_VERSION_MAJOR != 11
#error "Use the existing verified FFmpeg 8.x SDK"
#endif

typedef struct {
    AVBufferRef *device, *upload_pool;
    AVFilterGraph *graph;
    AVFilterContext *source, *sink;
    AVCodecContext *codec;
    AVBSFContext *metadata;
    AVFrame *raw, *upload, *scaled;
    AVPacket *packet;
    int width, height, pixel;
} WeaveloEncoder;

typedef struct {
    const uint8_t *data;
    size_t bytes;
    int64_t pts;
    uint64_t upload_vpp_us, encode_us;
    int key;
} WeaveloEncoded;

static uint64_t now_us(void) {
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return (uint64_t)t.tv_sec * 1000000 + (uint64_t)t.tv_nsec / 1000;
}

void weavelo_encoder_destroy(WeaveloEncoder *e) {
    if (!e) return;
    av_packet_free(&e->packet);
    av_frame_free(&e->raw);
    av_frame_free(&e->upload);
    av_frame_free(&e->scaled);
    av_bsf_free(&e->metadata);
    avcodec_free_context(&e->codec);
    avfilter_graph_free(&e->graph);
    av_buffer_unref(&e->upload_pool);
    av_buffer_unref(&e->device);
    free(e);
}

int weavelo_encoder_create(const char *device, const char *pixel, int width, int height,
    int output_width, int output_height, int fps, int bitrate, int range, int matrix,
    int transfer, int primaries, WeaveloEncoder **out) {
    if (!out || !device || !pixel || width < 16 || height < 16 || output_width < 16 ||
        output_height < 16 || fps < 1 || fps > 120 || bitrate < 1) return AVERROR(EINVAL);
    *out = NULL;
    if ((avcodec_version() >> 16) != 62 || (avutil_version() >> 16) != 60 ||
        (avfilter_version() >> 16) != 11) return AVERROR(ENOSYS);
    WeaveloEncoder *e = calloc(1, sizeof(*e));
    if (!e) return AVERROR(ENOMEM);
    int result = AVERROR(ENOMEM);
    e->width = width; e->height = height; e->pixel = av_get_pix_fmt(pixel);
    if (e->pixel == AV_PIX_FMT_NONE) { result = AVERROR(EINVAL); goto fail; }
    result = av_hwdevice_ctx_create(&e->device, AV_HWDEVICE_TYPE_VAAPI, device, NULL, 0);
    if (result < 0) goto fail;
    e->upload_pool = av_hwframe_ctx_alloc(e->device);
    if (!e->upload_pool) goto nomem;
    AVHWFramesContext *pool = (void *)e->upload_pool->data;
    pool->format = AV_PIX_FMT_VAAPI; pool->sw_format = e->pixel;
    pool->width = width; pool->height = height; pool->initial_pool_size = 2;
    result = av_hwframe_ctx_init(e->upload_pool);
    if (result < 0) goto fail;
    e->graph = avfilter_graph_alloc();
    if (!e->graph) goto nomem;
    e->graph->nb_threads = 1;
    char args[512];
    snprintf(args, sizeof(args), "video_size=%dx%d:pix_fmt=%d:time_base=1/%d:pixel_aspect=1/1",
        width, height, AV_PIX_FMT_VAAPI, fps);
    e->source = avfilter_graph_alloc_filter(e->graph, avfilter_get_by_name("buffer"), "input");
    if (!e->source) goto nomem;
    AVBufferSrcParameters *params = av_buffersrc_parameters_alloc();
    if (!params) goto nomem;
    params->format = AV_PIX_FMT_VAAPI; params->hw_frames_ctx = e->upload_pool;
    int rgb = av_pix_fmt_desc_get(e->pixel)->flags & AV_PIX_FMT_FLAG_RGB;
    params->color_space = rgb ? AVCOL_SPC_RGB : matrix;
    params->color_range = rgb ? AVCOL_RANGE_JPEG : range;
    result = av_buffersrc_parameters_set(e->source, params);
    av_free(params);
    if (result < 0) goto fail;
    result = avfilter_init_str(e->source, args);
    if (result < 0) goto fail;
    AVFilterContext *scale;
    snprintf(args, sizeof(args), "w=%d:h=%d:force_original_aspect_ratio=decrease:force_divisible_by=2:format=nv12:out_color_matrix=%s:out_range=%s:out_color_primaries=bt709:out_color_transfer=%s",
        output_width, output_height, matrix == 1 ? "bt709" : "smpte170m",
        range == 2 ? "full" : "limited", transfer == 13 ? "iec61966-2-1" : "bt709");
    result = avfilter_graph_create_filter(&scale, avfilter_get_by_name("scale_vaapi"), "scale", args, NULL, e->graph);
    if (result < 0) goto fail;
    result = avfilter_graph_create_filter(&e->sink, avfilter_get_by_name("buffersink"), "output", NULL, NULL, e->graph);
    if (result < 0) goto fail;
    if ((result = avfilter_link(e->source, 0, scale, 0)) < 0 ||
        (result = avfilter_link(scale, 0, e->sink, 0)) < 0 ||
        (result = avfilter_graph_config(e->graph, NULL)) < 0) goto fail;
    const AVCodec *codec = avcodec_find_encoder_by_name("hevc_vaapi");
    if (!codec) { result = AVERROR_ENCODER_NOT_FOUND; goto fail; }
    e->codec = avcodec_alloc_context3(codec);
    if (!e->codec) goto nomem;
    e->codec->width = av_buffersink_get_w(e->sink);
    e->codec->height = av_buffersink_get_h(e->sink);
    e->codec->pix_fmt = AV_PIX_FMT_VAAPI;
    e->codec->hw_frames_ctx = av_buffer_ref(av_buffersink_get_hw_frames_ctx(e->sink));
    if (!e->codec->hw_frames_ctx) goto nomem;
    e->codec->time_base = (AVRational){1, fps}; e->codec->framerate = (AVRational){fps, 1};
    e->codec->bit_rate = (int64_t)bitrate * 1000;
    e->codec->rc_max_rate = (int64_t)bitrate * 1000; e->codec->rc_buffer_size = bitrate * 1000;
    e->codec->gop_size = fps; e->codec->max_b_frames = 0; e->codec->thread_count = 1;
    e->codec->flags |= AV_CODEC_FLAG_LOW_DELAY;
    e->codec->color_range = range; e->codec->colorspace = matrix;
    e->codec->color_trc = transfer; e->codec->color_primaries = primaries;
    AVDictionary *options = NULL;
    av_dict_set(&options, "async_depth", "1", 0);
    result = avcodec_open2(e->codec, codec, &options);
    av_dict_free(&options);
    if (result < 0) goto fail;
    result = av_bsf_alloc(av_bsf_get_by_name("hevc_metadata"), &e->metadata);
    if (result < 0) goto fail;
    if ((result = avcodec_parameters_from_context(e->metadata->par_in, e->codec)) < 0) goto fail;
    e->metadata->time_base_in = e->codec->time_base;
    if ((result = av_opt_set(e->metadata->priv_data, "aud", "insert", 0)) < 0 ||
        (result = av_opt_set_int(e->metadata->priv_data, "video_full_range_flag", range == 2, 0)) < 0 ||
        (result = av_opt_set_int(e->metadata->priv_data, "colour_primaries", primaries, 0)) < 0 ||
        (result = av_opt_set_int(e->metadata->priv_data, "transfer_characteristics", transfer, 0)) < 0 ||
        (result = av_opt_set_int(e->metadata->priv_data, "matrix_coefficients", matrix, 0)) < 0 ||
        (result = av_bsf_init(e->metadata)) < 0) goto fail;
    e->raw = av_frame_alloc(); e->upload = av_frame_alloc(); e->scaled = av_frame_alloc();
    e->packet = av_packet_alloc();
    if (!e->raw || !e->upload || !e->scaled || !e->packet) goto nomem;
    e->raw->width = width; e->raw->height = height; e->raw->format = e->pixel;
    e->raw->color_range = rgb ? AVCOL_RANGE_JPEG : range;
    e->raw->colorspace = rgb ? AVCOL_SPC_RGB : matrix;
    e->raw->color_trc = transfer; e->raw->color_primaries = primaries;
    *out = e; return 0;
nomem:
    result = AVERROR(ENOMEM);
fail:
    weavelo_encoder_destroy(e); return result;
}

/* Input planes remain borrowed until upload returns. No frame pixels escape. */
int weavelo_encoder_frame(WeaveloEncoder *e, const uint8_t *const data[3],
    const int strides[3], int64_t pts, WeaveloEncoded *out) {
    if (!e || !data || !strides || !out) return AVERROR(EINVAL);
    for (int i = 0; i < 3; ++i) { e->raw->data[i] = (uint8_t *)data[i]; e->raw->linesize[i] = strides[i]; }
    e->raw->pts = pts;
    av_frame_unref(e->upload); av_frame_unref(e->scaled); av_packet_unref(e->packet);
    uint64_t started = now_us();
    int result = av_hwframe_get_buffer(e->upload_pool, e->upload, 0);
    if (result < 0) return result;
    if ((result = av_hwframe_transfer_data(e->upload, e->raw, 0)) < 0 ||
        (result = av_frame_copy_props(e->upload, e->raw)) < 0 ||
        (result = av_buffersrc_add_frame_flags(e->source, e->upload, 0)) < 0 ||
        (result = av_buffersink_get_frame(e->sink, e->scaled)) < 0) return result;
    uint64_t submitted = now_us();
    if ((result = avcodec_send_frame(e->codec, e->scaled)) < 0 ||
        (result = avcodec_receive_packet(e->codec, e->packet)) < 0 ||
        (result = av_bsf_send_packet(e->metadata, e->packet)) < 0 ||
        (result = av_bsf_receive_packet(e->metadata, e->packet)) < 0) return result;
    if (e->packet->pts != pts || e->packet->size <= 0 || e->packet->size > (32 << 20)) return AVERROR_INVALIDDATA;
    *out = (WeaveloEncoded){e->packet->data, (size_t)e->packet->size, e->packet->pts,
        submitted - started, now_us() - submitted, !!(e->packet->flags & AV_PKT_FLAG_KEY)};
    return 0;
}
