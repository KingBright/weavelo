package fun.hackerlife.weavelo;

import android.graphics.Canvas;
import android.graphics.Color;
import android.media.Image;
import android.media.MediaCodec;
import android.media.MediaCodecInfo;
import android.media.MediaFormat;
import java.nio.ByteBuffer;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.Map;
import kotlin.Unit;

/** Synthetic pixels only. Runs as an ADB shell process without installing an app. */
class AndroidGpuViewportProbe {
    static void require(boolean condition, String message) {
        if (!condition) throw new AssertionError(message);
    }
    static final class Encoded {
        final byte[] bytes;
        final long pts;
        Encoded(byte[] bytes, long pts) { this.bytes=bytes; this.pts=pts; }
    }
    static final class View implements AutoCloseable {
        final GpuVideoSurface gpu;
        final SurfaceHevcEncoder codec;
        final GpuVideoSurface.Output output;
        final ArrayList<Encoded> encoded=new ArrayList<>();
        final Map<Long,Integer> expected=new HashMap<>();
        final long sourceTime;
        boolean closed;
        View(GpuVideoSurface gpu) {
            this.gpu=gpu; sourceTime=gpu.getSourceCaptureUs();
            SurfaceHevcEncoder.Plan supported=SurfaceHevcEncoder.Companion.exact(256,128,30,1500);
            require(supported!=null,"No hardware HEVC fixture configuration");
            codec=new SurfaceHevcEncoder(new SurfaceHevcEncoder.Plan(supported.getCodec(),256,128,30,1500,true));
            output=gpu.addOutput(codec.getSurface());
            codec.requestKeyframe(1);
        }
        boolean receive(byte[] bytes, Long pts) {
            require(expected.containsKey(pts),"Codec changed the frame timestamp");
            encoded.add(new Encoded(bytes,pts));return true;
        }
        void draw(int x,int y,int gray,long pts) {
            require(gpu.getSourceCaptureUs()==sourceTime,"A static redraw invented a new capture time");
            expected.put(pts,gray);
            gpu.drawSource(output,new VideoFrameGeometry(0,pts,pts,641,371,x,y,256,128,256,128),pts);
            codec.drain(0L,this::receive);
        }
        void finish() {
            require(codec.finish(this::receive),"Encoder output was rejected");
            require(encoded.size()==expected.size(),"Encoder did not preserve all admitted frames");
        }
        public void close() { if (!closed) { closed=true;gpu.removeOutput(output);codec.close(); } }
    }
    static String decode(View view) throws Exception {
        MediaCodec decoder=MediaCodec.createDecoderByType(MediaFormat.MIMETYPE_VIDEO_HEVC);
        if (android.os.Build.VERSION.SDK_INT>=29)
            require(decoder.getCodecInfo().isHardwareAccelerated(),"Fixture did not use hardware HEVC decoding");
        MediaFormat format=MediaFormat.createVideoFormat(MediaFormat.MIMETYPE_VIDEO_HEVC,256,128);
        format.setInteger(MediaFormat.KEY_COLOR_FORMAT,MediaCodecInfo.CodecCapabilities.COLOR_FormatYUV420Flexible);
        decoder.configure(format,null,null,0);decoder.start();
        int next=0,decoded=0,range=0;
        boolean eos=false,done=false;
        String metadata="";
        ArrayList<Integer> observed=new ArrayList<>();
        MediaCodec.BufferInfo info=new MediaCodec.BufferInfo();
        long deadline=System.nanoTime()+5_000_000_000L;
        try {
            while (!done && System.nanoTime()<deadline) {
                if (!eos) {
                    int input=decoder.dequeueInputBuffer(1000);
                    if (input>=0) {
                        if (next<view.encoded.size()) {
                            Encoded frame=view.encoded.get(next++);
                            ByteBuffer buffer=decoder.getInputBuffer(input);buffer.clear();buffer.put(frame.bytes);
                            decoder.queueInputBuffer(input,0,frame.bytes.length,frame.pts,0);
                        } else {
                            decoder.queueInputBuffer(input,0,0,0,MediaCodec.BUFFER_FLAG_END_OF_STREAM);eos=true;
                        }
                    }
                }
                int index=decoder.dequeueOutputBuffer(info,1000);
                if (index==MediaCodec.INFO_OUTPUT_FORMAT_CHANGED) {
                    MediaFormat actual=decoder.getOutputFormat();metadata=actual.toString();
                    if (actual.containsKey(MediaFormat.KEY_COLOR_RANGE)) range=actual.getInteger(MediaFormat.KEY_COLOR_RANGE);
                } else if (index>=0) {
                    try {
                        Image image=decoder.getOutputImage(index);
                        if (image!=null) {
                            try {
                                Integer gray=view.expected.get(info.presentationTimeUs);
                                require(gray!=null,"Decoder lost frame geometry ownership");
                                Image.Plane plane=image.getPlanes()[0];
                                android.graphics.Rect crop=image.getCropRect();
                                int offset=(crop.top+crop.height()/2)*plane.getRowStride()
                                    +(crop.left+crop.width()/2)*plane.getPixelStride();
                                int y=plane.getBuffer().get(offset)&255;
                                int target=range==MediaFormat.COLOR_RANGE_LIMITED ? 16+(gray*219+127)/255 : gray;
                                require(Math.abs(y-target)<=10,"GPU crop sampled the wrong source region: expected="+target+" actual="+y);
                                observed.add(y);decoded++;
                            } finally { image.close(); }
                        }
                        done=(info.flags&MediaCodec.BUFFER_FLAG_END_OF_STREAM)!=0;
                    } finally { decoder.releaseOutputBuffer(index,false); }
                }
            }
            require(done && decoded==view.expected.size(),"Decoder did not present every fixture image");
            return "{\"frames\":"+decoded+",\"center_y\":"+observed+",\"color_range\":"+range+",\"codec\":\""+decoder.getName()+"\"}";
        } finally {
            System.err.println("WEAVELO_GPU_DECODE_FORMAT "+metadata);
            decoder.stop();decoder.release();
        }
    }
    static SurfaceHevcEncoder.Plan verifyOutgoingPlan(int width,int height,int fps,int bitrate) {
        SurfaceHevcEncoder.Plan plan=SurfaceHevcEncoder.Companion.fitAtMost(1080,1920,width,height,fps,bitrate);
        require(plan!=null,"No hardware encoder plan for outgoing quality");
        require(plan.getWidth()<=width && plan.getHeight()<=height && plan.getFps()<=fps &&
            plan.getBitrateKbps()<=bitrate,"Hardware plan exceeds the outgoing quality bounds");
        try (SurfaceHevcEncoder encoder=new SurfaceHevcEncoder(plan)) {
            require(encoder.getSurface().isValid(),"Outgoing hardware encoder has no valid surface");
        }
        return plan;
    }
    public static void main(String[] args) throws Exception {
        SurfaceHevcEncoder.Plan low=verifyOutgoingPlan(320,568,30,2500);
        SurfaceHevcEncoder.Plan high=verifyOutgoingPlan(720,1280,60,10000);
        require(high.getWidth()>low.getWidth() && high.getHeight()>low.getHeight(),
            "Outgoing quality levels did not produce distinct hardware sizes");
        String quality="[["+low.getWidth()+","+low.getHeight()+","+low.getFps()+","+low.getBitrateKbps()+"],["+
            high.getWidth()+","+high.getHeight()+","+high.getFps()+","+high.getBitrateKbps()+"]]";
        System.err.println("WEAVELO_GPU_STEP create-context");
        GpuVideoSurface gpu=new GpuVideoSurface(null,641,371,()->System.nanoTime()/1000,0,g->Unit.INSTANCE);
        try {
            System.err.println("WEAVELO_GPU_STEP draw-synthetic-source");
            Canvas canvas=gpu.getInput().lockHardwareCanvas();
            try {
                int[] grays={32,96,160,224};
                for(int i=0;i<4;i++) {
                    canvas.save();canvas.clipRect((i%2)*321,(i/2)*186,(i%2==0)?321:641,(i<2)?186:371);
                    canvas.drawColor(Color.rgb(grays[i],grays[i],grays[i]));canvas.restore();
                }
            } finally { gpu.getInput().unlockCanvasAndPost(canvas); }
            long until=System.nanoTime()+2_000_000_000L;
            while (!gpu.latchSource() && System.nanoTime()<until) gpu.awaitSource(10);
            require(gpu.getSourceReady(),"Synthetic raw texture never arrived");
            require(!gpu.latchSource(),"Idle capture unexpectedly acquired a new image");
            System.err.println("WEAVELO_GPU_STEP create-encoders dataSpace="+gpu.getSourceDataSpace());
            View a=new View(gpu),b=new View(gpu);
            try {
                a.draw(0,0,32,1000);b.draw(0,220,160,2000);
                a.draw(384,0,96,3000);b.draw(384,220,224,4000);
                System.err.println("WEAVELO_GPU_STEP drain-first-encoder");
                a.finish();System.err.println("WEAVELO_GPU_STEP decode-first-stream");String first=decode(a);a.close();
                b.codec.requestKeyframe(2);b.draw(0,0,32,5000);
                b.finish();String second=decode(b);
                System.out.println("WEAVELO_GPU_VIEWPORT_PASS {\"first\":"+first+",\"second\":"+second+
                    ",\"same_capture_time\":true,\"independent_stop\":true,\"source_size\":[641,371],\"data_space\":"+gpu.getSourceDataSpace()+
                    ",\"outgoing_hardware_plans\":"+quality+"}");
            } finally { a.close();b.close(); }
        } finally { gpu.close(); }
        // app_process starts framework Binder threads outside an Activity
        // lifecycle. End this fixture process after releasing native owners.
        System.exit(0);
    }
}
