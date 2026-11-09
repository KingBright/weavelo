// CoreAudio process tap. No default-device, hardware volume, or mute changes.
#import <Foundation/Foundation.h>
#import <CoreAudio/CoreAudio.h>
#import <CoreAudio/AudioHardwareTapping.h>
#import <CoreAudio/CATapDescription.h>
#import <AudioToolbox/AudioToolbox.h>
#import <errno.h>
#import <stdatomic.h>
#import <stdbool.h>
#import <math.h>
#import <stdint.h>
#import <stdio.h>
#import <stdlib.h>
#import <time.h>
#import <unistd.h>

#define TAP_CAPACITY 32768u

typedef struct {
    AudioObjectID tap;
    AudioDeviceID aggregate;
    CFStringRef aggregate_uid;
    AudioDeviceIOProcID proc;
    AudioStreamBasicDescription format;
    _Atomic uint64_t write_pos;
    _Atomic uint64_t read_pos;
    float samples[TAP_CAPACITY * 2];
    float converter_input[2048];
    AudioConverterRef converter;
    UInt32 capacity;
    _Atomic bool overflow;
    bool running;
    bool aggregate_destroy_requested;
} WeaveloAudioTap;

static void tap_error(char *error, size_t cap, const char *step, OSStatus status) {
    if (error && cap) snprintf(error, cap, "%s: CoreAudio OSStatus %d%s", step, (int)status,
        status == kAudioDevicePermissionsError ? " (audio device permission denied)" : "");
}

static OSStatus process_object(pid_t pid, AudioObjectID *object) {
    AudioObjectPropertyAddress address = { kAudioHardwarePropertyTranslatePIDToProcessObject,
        kAudioObjectPropertyScopeGlobal, kAudioObjectPropertyElementMain };
    UInt32 size = sizeof(*object);
    *object = kAudioObjectUnknown;
    OSStatus s = AudioObjectGetPropertyData(kAudioObjectSystemObject, &address, sizeof(pid), &pid, &size, object);
    return s == noErr && *object == kAudioObjectUnknown ? kAudioHardwareBadObjectError : s;
}

static OSStatus tap_io(AudioObjectID device, const AudioTimeStamp *now,
                       const AudioBufferList *input, const AudioTimeStamp *input_time,
                       AudioBufferList *output, const AudioTimeStamp *output_time, void *context) {
    (void)device; (void)now; (void)input_time; (void)output; (void)output_time;
    WeaveloAudioTap *t = context;
    if (!input || !input->mNumberBuffers) return noErr;
    bool planar = (t->format.mFormatFlags & kAudioFormatFlagIsNonInterleaved) != 0;
    if ((planar && (input->mNumberBuffers < 2 || !input->mBuffers[0].mData || !input->mBuffers[1].mData)) ||
        (!planar && (!input->mBuffers[0].mData || input->mBuffers[0].mNumberChannels != 2))) return noErr;
    UInt32 frames = input->mBuffers[0].mDataByteSize / (planar ? sizeof(float) : 2 * sizeof(float));
    if (planar) {
        UInt32 right_frames = input->mBuffers[1].mDataByteSize / sizeof(float);
        if (right_frames < frames) frames = right_frames;
    }
    const float *left = input->mBuffers[0].mData;
    const float *right = planar ? input->mBuffers[1].mData : left;
    uint64_t w = atomic_load_explicit(&t->write_pos, memory_order_relaxed);
    uint64_t r = atomic_load_explicit(&t->read_pos, memory_order_acquire);
    uint64_t available = t->capacity - (w - r);
    if (frames > available) {
        atomic_store_explicit(&t->overflow, true, memory_order_release);
        frames = (UInt32)available; // bounded; never wait in realtime callback
    }
    for (UInt32 i = 0; i < frames; ++i) {
        size_t slot = ((w + i) % TAP_CAPACITY) * 2;
        t->samples[slot] = left[planar ? i : 2 * i];
        t->samples[slot + 1] = right[planar ? i : 2 * i + 1];
    }
    atomic_store_explicit(&t->write_pos, w + frames, memory_order_release);
    return noErr;
}

// Query the system device list and verify the aggregate's unique UID. Device
// IDs can be reused, so an ID match alone is not proof that our aggregate lives.
static OSStatus tap_aggregate_is_present(WeaveloAudioTap *t, bool *present) {
    *present = false;
    if (t->aggregate == kAudioObjectUnknown) return noErr;
    if (!t->aggregate_uid) return kAudioHardwareUnspecifiedError;

    AudioObjectPropertyAddress devices_address = {
        kAudioHardwarePropertyDevices,
        kAudioObjectPropertyScopeGlobal,
        kAudioObjectPropertyElementMain
    };
    UInt32 bytes = 0;
    OSStatus s = AudioObjectGetPropertyDataSize(kAudioObjectSystemObject,
        &devices_address, 0, NULL, &bytes);
    if (s != noErr) return s;
    if (bytes % sizeof(AudioObjectID) != 0) return kAudioHardwareUnspecifiedError;
    if (bytes == 0) return noErr;

    AudioObjectID *devices = calloc(bytes / sizeof(AudioObjectID), sizeof(AudioObjectID));
    if (!devices) return kAudioHardwareUnspecifiedError;
    UInt32 capacity = bytes;
    s = AudioObjectGetPropertyData(kAudioObjectSystemObject, &devices_address,
        0, NULL, &bytes, devices);
    if (s != noErr || bytes > capacity || bytes % sizeof(AudioObjectID) != 0) {
        free(devices);
        return s != noErr ? s : kAudioHardwareBadPropertySizeError;
    }

    UInt32 count = bytes / sizeof(AudioObjectID);
    for (UInt32 i = 0; i < count; ++i) {
        if (devices[i] != t->aggregate) continue;
        AudioObjectPropertyAddress uid_address = {
            kAudioDevicePropertyDeviceUID,
            kAudioObjectPropertyScopeGlobal,
            kAudioObjectPropertyElementMain
        };
        CFStringRef current_uid = NULL;
        UInt32 uid_size = sizeof(current_uid);
        s = AudioObjectGetPropertyData(devices[i], &uid_address, 0, NULL,
            &uid_size, &current_uid);
        if (s == noErr && uid_size == sizeof(current_uid) && current_uid) {
            *present = CFEqual(current_uid, t->aggregate_uid);
            CFRelease(current_uid);
        } else if (s == noErr) {
            s = kAudioHardwareUnspecifiedError;
        }
        break;
    }
    free(devices);
    return s;
}

// Aggregate destruction is asynchronous. Keep its ID and UID until the exact
// device disappears; a timeout leaves the native owner intact for a later retry.
static OSStatus tap_destroy_aggregate_and_wait(WeaveloAudioTap *t) {
    if (t->aggregate == kAudioObjectUnknown) return noErr;
    bool present = false;
    OSStatus s = tap_aggregate_is_present(t, &present);
    if (s != noErr) return s;
    if (!present) {
        t->aggregate = kAudioObjectUnknown;
        return noErr;
    }

    if (!t->aggregate_destroy_requested) {
        s = AudioHardwareDestroyAggregateDevice(t->aggregate);
        if (s != noErr) {
            OSStatus observe = tap_aggregate_is_present(t, &present);
            if (observe != noErr) return observe;
            if (!present) {
                t->aggregate = kAudioObjectUnknown;
                return noErr;
            }
            return s;
        }
        t->aggregate_destroy_requested = true;
    }

    struct timespec deadline;
    if (clock_gettime(CLOCK_MONOTONIC, &deadline) != 0)
        return kAudioHardwareUnspecifiedError;
    deadline.tv_nsec += 250 * 1000 * 1000;
    if (deadline.tv_nsec >= 1000 * 1000 * 1000) {
        deadline.tv_sec += 1;
        deadline.tv_nsec -= 1000 * 1000 * 1000;
    }

    for (;;) {
        s = tap_aggregate_is_present(t, &present);
        if (s != noErr) return s;
        if (!present) {
            t->aggregate = kAudioObjectUnknown;
            return noErr;
        }

        struct timespec now;
        if (clock_gettime(CLOCK_MONOTONIC, &now) != 0)
            return kAudioHardwareUnspecifiedError;
        if (now.tv_sec > deadline.tv_sec ||
            (now.tv_sec == deadline.tv_sec && now.tv_nsec >= deadline.tv_nsec)) {
            fprintf(stderr, "Weavelo: aggregate removal not observed; retaining tap owner for retry\n");
            return kAudioHardwareUnspecifiedError;
        }
        struct timespec delay = { .tv_sec = 0, .tv_nsec = 10 * 1000 * 1000 };
        int sleep_result;
        do {
            sleep_result = nanosleep(&delay, &delay);
        } while (sleep_result != 0 && errno == EINTR);
        if (sleep_result != 0) return kAudioHardwareUnspecifiedError;
    }
}

// A failed native teardown leaves its handle owned by the caller. In particular,
// neither the callback nor its context may be destroyed after a failed stop.
static OSStatus tap_dispose(WeaveloAudioTap *t) {
    if (!t) return noErr;
    if (t->running) {
        OSStatus s = AudioDeviceStop(t->aggregate, t->proc);
        if (s != noErr) return s;
        t->running = false;
    }
    if (t->proc) {
        OSStatus s = AudioDeviceDestroyIOProcID(t->aggregate, t->proc);
        if (s != noErr) return s;
        t->proc = NULL;
    }
    if (t->aggregate != kAudioObjectUnknown) {
        OSStatus s = tap_destroy_aggregate_and_wait(t);
        if (s != noErr) return s;
    }
    if (t->tap != kAudioObjectUnknown) {
        OSStatus s = kAudioHardwareUnsupportedOperationError;
        if (@available(macOS 14.2, *)) s = AudioHardwareDestroyProcessTap(t->tap);
        if (s != noErr) return s;
        t->tap = kAudioObjectUnknown;
    }
    if (t->converter) {
        OSStatus s = AudioConverterDispose(t->converter);
        if (s != noErr) return s;
        t->converter = NULL;
    }
    if (t->aggregate_uid) {
        CFRelease(t->aggregate_uid);
        t->aggregate_uid = NULL;
    }
    free(t);
    return noErr;
}

static OSStatus tap_make_converter(WeaveloAudioTap *t) {
    AudioStreamBasicDescription input = {0};
    input.mSampleRate = t->format.mSampleRate;
    input.mFormatID = kAudioFormatLinearPCM;
    input.mFormatFlags = kAudioFormatFlagsNativeFloatPacked;
    input.mBytesPerPacket = input.mBytesPerFrame = 2 * sizeof(float);
    input.mFramesPerPacket = 1;
    input.mChannelsPerFrame = 2;
    input.mBitsPerChannel = 32;
    AudioStreamBasicDescription output = input;
    output.mSampleRate = 48000.0;
    return AudioConverterNew(&input, &output, &t->converter);
}

bool weavelo_audio_tap_available(void) {
    if (@available(macOS 14.2, *)) return true;
    return false;
}

// A partial owner is returned even on creation failure. Its Rust owner
// tracks actual teardown before releasing the host-wide audio claim.
void *weavelo_audio_tap_create(bool *ready, char *error, size_t cap) {
    *ready = false;
    @autoreleasepool {
        if (@available(macOS 14.2, *)) {
            WeaveloAudioTap *t = calloc(1, sizeof(*t));
            if (!t) { if (error && cap) snprintf(error, cap, "audio tap allocation failed"); return NULL; }
            AudioObjectID process = kAudioObjectUnknown;
            OSStatus s = process_object(getpid(), &process);
            if (s != noErr) { tap_error(error, cap, "resolve audio process", s); free(t); return NULL; }
            CATapDescription *desc =
                [[CATapDescription alloc] initStereoGlobalTapButExcludeProcesses:@[@(process)]];
            desc.name = @"Weavelo audio capture";
            desc.UUID = [NSUUID UUID];
            desc.privateTap = YES;
            desc.muteBehavior = CATapUnmuted;
            s = AudioHardwareCreateProcessTap(desc, &t->tap);
            if (s != noErr) { tap_error(error, cap, "create process tap", s); return t; }
            AudioObjectPropertyAddress fmt = { kAudioTapPropertyFormat, kAudioObjectPropertyScopeGlobal, kAudioObjectPropertyElementMain };
            UInt32 len = sizeof(t->format);
            s = AudioObjectGetPropertyData(t->tap, &fmt, 0, NULL, &len, &t->format);
            if (s != noErr) { tap_error(error, cap, "read tap format", s); return t; }
            AudioStreamBasicDescription f = t->format;
            if (f.mFormatID != kAudioFormatLinearPCM || !(f.mFormatFlags & kAudioFormatFlagIsFloat) ||
                (f.mFormatFlags & kAudioFormatFlagIsBigEndian) || f.mBitsPerChannel != 32 ||
                f.mChannelsPerFrame != 2 || !isfinite(f.mSampleRate) || f.mSampleRate < 8000 || f.mSampleRate > 192000 ||
                f.mFramesPerPacket != 1 || f.mBytesPerFrame != ((f.mFormatFlags & kAudioFormatFlagIsNonInterleaved) ? 4 : 8)) {
                if (error && cap) snprintf(error, cap, "unsupported CoreAudio tap PCM format");
                return t;
            }
            t->capacity = (UInt32)ceil(f.mSampleRate * 0.1);
            if (t->capacity > TAP_CAPACITY) t->capacity = TAP_CAPACITY;
            s = tap_make_converter(t);
            if (s != noErr) { tap_error(error, cap, "create native audio converter", s); return t; }
            NSString *uid = [[NSUUID UUID] UUIDString];
            t->aggregate_uid = CFRetain((__bridge CFStringRef)uid);
            NSDictionary *composition = @{
                @kAudioAggregateDeviceNameKey: @"Weavelo private audio tap",
                @kAudioAggregateDeviceUIDKey: uid,
                @kAudioAggregateDeviceIsPrivateKey: @YES,
                @kAudioAggregateDeviceTapListKey: @[@{@kAudioSubTapUIDKey: desc.UUID.UUIDString,
                                                       @kAudioSubTapDriftCompensationKey: @YES}]
            };
            s = AudioHardwareCreateAggregateDevice((__bridge CFDictionaryRef)composition, &t->aggregate);
            if (s != noErr) { tap_error(error, cap, "create private aggregate", s); return t; }
            s = AudioDeviceCreateIOProcID(t->aggregate, tap_io, t, &t->proc);
            if (s != noErr) { tap_error(error, cap, "create audio callback", s); return t; }
            *ready = true;
            return t;
        }
        if (error && cap) snprintf(error, cap, "CoreAudio process taps require macOS 14.2 or later");
        return NULL;
    }
}

int32_t weavelo_audio_tap_start(void *handle) {
    WeaveloAudioTap *t = handle;
    if (!t) return kAudioHardwareBadObjectError;
    if (t->running) return noErr;
    OSStatus s = AudioDeviceStart(t->aggregate, t->proc);
    if (s == noErr) t->running = true;
    return s;
}

int32_t weavelo_audio_tap_set_private(void *handle, bool private_audio) {
    @autoreleasepool {
        if (@available(macOS 14.2, *)) {
        WeaveloAudioTap *t = handle;
        if (!t) return kAudioHardwareBadObjectError;
        AudioObjectPropertyAddress address = { kAudioTapPropertyDescription, kAudioObjectPropertyScopeGlobal, kAudioObjectPropertyElementMain };
        CATapDescription *desc = nil;
        UInt32 size = sizeof(desc);
        OSStatus s = AudioObjectGetPropertyData(t->tap, &address, 0, NULL, &size, &desc);
        if (s != noErr || !desc) return s != noErr ? s : kAudioHardwareUnspecifiedError;
        desc.muteBehavior = private_audio ? CATapMutedWhenTapped : CATapUnmuted;
        s = AudioObjectSetPropertyData(t->tap, &address, 0, NULL, sizeof(desc), &desc);
        if (s != noErr) return s;
        // HAL changes can be applied asynchronously. Accept only readback
        // of the requested mode, with a bounded wait off the async executor.
        for (unsigned attempt = 0; attempt < 20; ++attempt) {
            CATapDescription *confirmed = nil;
            size = sizeof(confirmed);
            s = AudioObjectGetPropertyData(t->tap, &address, 0, NULL, &size, &confirmed);
            if (s != noErr) return s;
            if (confirmed && confirmed.muteBehavior == desc.muteBehavior) return noErr;
            struct timespec delay = { .tv_nsec = 5 * 1000 * 1000 };
            nanosleep(&delay, NULL);
        }
        return kAudioHardwareUnspecifiedError;
        }
        return kAudioHardwareUnsupportedOperationError;
    }
}

double weavelo_audio_tap_rate(void *handle) {
    WeaveloAudioTap *t = handle;
    AudioObjectPropertyAddress fmt = { kAudioTapPropertyFormat, kAudioObjectPropertyScopeGlobal, kAudioObjectPropertyElementMain };
    AudioStreamBasicDescription current = {0};
    UInt32 size = sizeof(current);
    if (AudioObjectGetPropertyData(t->tap, &fmt, 0, NULL, &size, &current) != noErr ||
        current.mSampleRate != t->format.mSampleRate || current.mFormatID != t->format.mFormatID ||
        current.mFormatFlags != t->format.mFormatFlags || current.mBytesPerFrame != t->format.mBytesPerFrame ||
        current.mChannelsPerFrame != t->format.mChannelsPerFrame || current.mBitsPerChannel != t->format.mBitsPerChannel)
        return 0;
    return current.mSampleRate;
}
// The converter input callback runs on the consumer's thread, never on the IOProc.
#define TAP_NO_DATA ((OSStatus)0x6e6f6474) /* 'nodt' */
static OSStatus tap_converter_input(AudioConverterRef converter, UInt32 *packets,
                                    AudioBufferList *data, AudioStreamPacketDescription **descriptions,
                                    void *context) {
    (void)converter; (void)descriptions;
    WeaveloAudioTap *t = context;
    uint64_t r = atomic_load_explicit(&t->read_pos, memory_order_relaxed);
    uint64_t w = atomic_load_explicit(&t->write_pos, memory_order_acquire);
    UInt32 count = (UInt32)(w - r);
    if (count > *packets) count = *packets;
    if (count > 1024) count = 1024;
    if (!count) { *packets = 0; return TAP_NO_DATA; }
    for (UInt32 i = 0; i < count; ++i) {
        size_t slot = ((r + i) % TAP_CAPACITY) * 2;
        t->converter_input[i * 2] = t->samples[slot];
        t->converter_input[i * 2 + 1] = t->samples[slot + 1];
    }
    atomic_store_explicit(&t->read_pos, r + count, memory_order_release);
    *packets = count;
    data->mNumberBuffers = 1;
    data->mBuffers[0].mNumberChannels = 2;
    data->mBuffers[0].mDataByteSize = count * 2 * sizeof(float);
    data->mBuffers[0].mData = t->converter_input;
    return noErr;
}

// Discontinuities are dropped and converter history cleared rather than emitting
// old audio after a stalled or paused consumer.
int32_t weavelo_audio_tap_clear(void *handle) {
    WeaveloAudioTap *t = handle;
    uint64_t w = atomic_load_explicit(&t->write_pos, memory_order_acquire);
    atomic_store_explicit(&t->read_pos, w, memory_order_release);
    atomic_store_explicit(&t->overflow, false, memory_order_release);
    return AudioConverterReset(t->converter);
}
int32_t weavelo_audio_tap_read(void *handle, float *out, uint32_t max_frames, uint32_t *produced) {
    WeaveloAudioTap *t = handle;
    *produced = 0;
    if (atomic_exchange_explicit(&t->overflow, false, memory_order_acq_rel)) {
        OSStatus s = weavelo_audio_tap_clear(t);
        if (s != noErr) return s;
        return noErr;
    }
    if (atomic_load_explicit(&t->write_pos, memory_order_acquire) ==
        atomic_load_explicit(&t->read_pos, memory_order_relaxed)) return noErr;
    AudioBufferList output = { .mNumberBuffers = 1,
        .mBuffers = {{ .mNumberChannels = 2, .mDataByteSize = max_frames * 2 * sizeof(float), .mData = out }} };
    UInt32 count = max_frames;
    OSStatus s = AudioConverterFillComplexBuffer(t->converter, tap_converter_input, t, &count, &output, NULL);
    if (count > max_frames) return kAudioHardwareBadPropertySizeError;
    *produced = count;
    return s == TAP_NO_DATA ? noErr : s;
}
int32_t weavelo_audio_tap_destroy(void *handle) { return tap_dispose(handle); }
