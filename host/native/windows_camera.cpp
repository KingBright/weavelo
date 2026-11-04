// All capture/codec COM owners live in a dedicated MTA. Only compressed bytes
// cross the Rust boundary. No camera pixels are mapped or converted on the CPU.
#include <windows.h>
#include <mfapi.h>
#include <mfidl.h>
#include <mfreadwrite.h>
#include <mftransform.h>
#include <mferror.h>
#include <codecapi.h>
#include <icodecapi.h>
#include <d3d11.h>
#include <d3d10.h>
#include <cfgmgr32.h>
#include <ks.h>
#include <wrl.h>
#include <atomic>
#include <chrono>
#include <condition_variable>
#include <cstdio>
#include <deque>
#include <memory>
#include <mutex>
#include <string>
#include <thread>
#include <vector>
#include <algorithm>
using Microsoft::WRL::ComPtr;
using Microsoft::WRL::RuntimeClass;
using Microsoft::WRL::RuntimeClassFlags;
using Microsoft::WRL::ClassicCom;
using Microsoft::WRL::FtmBase;
using Microsoft::WRL::Make;
using Clock=std::chrono::steady_clock;
using Packet=void(*)(void*,const unsigned char*,size_t,unsigned long long,unsigned long long);
using Error=void(*)(void*,const char*);
using Timestamp=unsigned long long(*)(void*);
using Admit=bool(*)(void*,unsigned long long);
using Format=void(*)(void*,UINT32,UINT32,UINT32,UINT32,UINT32,bool);
static std::atomic<unsigned long long> device_epoch{1};
static DWORD CALLBACK devices_changed(HCMNOTIFICATION,void*,CM_NOTIFY_ACTION,PCM_NOTIFY_EVENT_DATA,DWORD) {
    ++device_epoch;return ERROR_SUCCESS;
}
extern "C" unsigned long long weavelo_windows_camera_epoch() {
    static std::once_flag once;static HCMNOTIFICATION notification=nullptr;static HCMNOTIFICATION capture_notification=nullptr;
    std::call_once(once,[]{
        CM_NOTIFY_FILTER filter{};filter.cbSize=sizeof(filter);
        filter.FilterType=CM_NOTIFY_FILTER_TYPE_DEVICEINTERFACE;filter.u.DeviceInterface.ClassGuid=KSCATEGORY_VIDEO_CAMERA;
        bool ok=CM_Register_Notification(&filter,nullptr,devices_changed,&notification)==CR_SUCCESS;
        filter.u.DeviceInterface.ClassGuid=KSCATEGORY_CAPTURE;
        ok|=CM_Register_Notification(&filter,nullptr,devices_changed,&capture_notification)==CR_SUCCESS;
        if(!ok)device_epoch=0;
    });return (notification||capture_notification)?device_epoch.load():0;
}
struct Failure {HRESULT hr;const char* stage;};
static void check(HRESULT hr,const char* stage) {if(FAILED(hr))throw Failure{hr,stage};}
struct Apartment {
    Apartment(){check(CoInitializeEx(nullptr,COINIT_MULTITHREADED),"COM MTA");}
    ~Apartment(){CoUninitialize();}
};
struct MediaFoundation {
    MediaFoundation(){check(MFStartup(MF_VERSION,MFSTARTUP_FULL),"MFStartup");}
    ~MediaFoundation(){MFShutdown();}
};
struct Activations {
    IMFActivate** items=nullptr;UINT32 count=0;
    ~Activations(){for(UINT32 i=0;i<count;++i)items[i]->Release();CoTaskMemFree(items);}
};
static std::string utf8(const wchar_t* s) {
    int n=WideCharToMultiByte(CP_UTF8,WC_ERR_INVALID_CHARS,s,-1,nullptr,0,nullptr,nullptr);
    if(n<=0)throw Failure{E_INVALIDARG,"Camera device identity"};
    std::string r(n,'\0');WideCharToMultiByte(CP_UTF8,WC_ERR_INVALID_CHARS,s,-1,r.data(),n,nullptr,nullptr);r.pop_back();return r;
}
static std::wstring wide(const char* s) {
    int n=MultiByteToWideChar(CP_UTF8,MB_ERR_INVALID_CHARS,s,-1,nullptr,0);
    if(n<=0)throw Failure{E_INVALIDARG,"Camera device identity"};
    std::wstring r(n,L'\0');MultiByteToWideChar(CP_UTF8,MB_ERR_INVALID_CHARS,s,-1,r.data(),n);r.pop_back();return r;
}
static std::string text(IMFAttributes* a,const GUID& key) {
    wchar_t* s=nullptr;UINT32 size=0;check(a->GetAllocatedString(key,&s,&size),"Camera device name");
    std::unique_ptr<wchar_t,decltype(&CoTaskMemFree)> owner(s,CoTaskMemFree);return utf8(s);
}
static void enumerate(Activations& list) {
    ComPtr<IMFAttributes> attrs;check(MFCreateAttributes(&attrs,1),"Camera attributes");
    attrs->SetGUID(MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID);
    check(MFEnumDeviceSources(attrs.Get(),&list.items,&list.count),"Camera enumeration");
    if(list.count>32)throw Failure{E_BOUNDS,"Too many camera devices"};
}
static void report(Error error,void* context,Failure f) {
    char message[256];snprintf(message,sizeof(message),"Windows camera: %s (0x%08lx)",f.stage,static_cast<unsigned long>(f.hr));
    error(context,message);
}
extern "C" int weavelo_windows_camera_catalog(void* context,void(*item)(void*,const char*,const char*),Error error) {
    int status=0;
    try {
        std::thread([&]{
            try {
                Apartment apartment;MediaFoundation mf;Activations list;enumerate(list);
                for(UINT32 i=0;i<list.count;++i) {
                    auto key=text(list.items[i],MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK);
                    auto name=text(list.items[i],MF_DEVSOURCE_ATTRIBUTE_FRIENDLY_NAME);
                    item(context,key.c_str(),name.c_str());
                }
                status=1;
            } catch(Failure f){report(error,context,f);}catch(...){error(context,"Windows camera enumeration failed");}
        }).join();
    } catch(...){error(context,"Windows camera enumeration worker failed");}
    return status;
}

struct Raw {ComPtr<IMFSample> sample;unsigned long long captured=0;};
struct Shared {
    std::atomic<bool> stop{false},keyframe{false};
    std::mutex mutex;std::condition_variable wake;
    Raw latest;bool read_pending=false;HRESULT failure=S_OK;
    void* context;Timestamp timestamp;
    Shared(void* context,Timestamp timestamp):context(context),timestamp(timestamp){}
};
class ReaderCallback final:public RuntimeClass<RuntimeClassFlags<ClassicCom>,IMFSourceReaderCallback,FtmBase> {
    std::weak_ptr<Shared> shared;
public:
    ReaderCallback(std::shared_ptr<Shared> s):shared(s){}
    STDMETHODIMP OnReadSample(HRESULT status,DWORD,DWORD flags,LONGLONG,IMFSample* sample) override {
        if(auto s=shared.lock()) {
            Raw old;
            {std::lock_guard<std::mutex> lock(s->mutex);s->read_pending=false;
                if(FAILED(status))s->failure=status;
                else if(flags&(MF_SOURCE_READERF_ENDOFSTREAM|MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED|MF_SOURCE_READERF_NATIVEMEDIATYPECHANGED))s->failure=MF_E_INVALIDMEDIATYPE;
                else if(sample&&!s->stop) {old=std::move(s->latest);s->latest={sample,s->timestamp(s->context)};}
            }s->wake.notify_one();
        }return S_OK;
    }
    STDMETHODIMP OnFlush(DWORD) override {return S_OK;}
    STDMETHODIMP OnEvent(DWORD,IMFMediaEvent* event) override {
        HRESULT status=S_OK;HRESULT result=event?event->GetStatus(&status):E_POINTER;
        if(FAILED(result))status=result;
        if(FAILED(status))if(auto s=shared.lock()) {
            {std::lock_guard<std::mutex> lock(s->mutex);s->failure=status;}
            s->wake.notify_one();
        }return S_OK;
    }
};
struct Device {
    ComPtr<ID3D11Device> device;ComPtr<ID3D11DeviceContext> context;ComPtr<IMFDXGIDeviceManager> manager;
    Device() {
        D3D_FEATURE_LEVEL level;
        check(D3D11CreateDevice(nullptr,D3D_DRIVER_TYPE_HARDWARE,nullptr,D3D11_CREATE_DEVICE_VIDEO_SUPPORT|D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            nullptr,0,D3D11_SDK_VERSION,&device,&level,&context),"Hardware D3D11 device");
        ComPtr<ID3D10Multithread> multi;check(context.As(&multi),"D3D11 multithread protection");multi->SetMultithreadProtected(TRUE);
        UINT token;check(MFCreateDXGIDeviceManager(&token,&manager),"DXGI manager");check(manager->ResetDevice(device.Get(),token),"DXGI device");
    }
};
struct Signal {UINT32 matrix,primaries,transfer,range;};
static Signal signal(IMFMediaType* type) {
    Signal s{};check(type->GetUINT32(MF_MT_YUV_MATRIX,&s.matrix),"Missing camera YUV matrix");
    check(type->GetUINT32(MF_MT_VIDEO_PRIMARIES,&s.primaries),"Missing camera primaries");
    check(type->GetUINT32(MF_MT_TRANSFER_FUNCTION,&s.transfer),"Missing camera transfer");
    check(type->GetUINT32(MF_MT_VIDEO_NOMINAL_RANGE,&s.range),"Missing camera range");
    if((s.matrix!=MFVideoTransferMatrix_BT709&&s.matrix!=MFVideoTransferMatrix_BT601)||s.primaries!=MFVideoPrimaries_BT709||
        (s.transfer!=MFVideoTransFunc_709&&s.transfer!=MFVideoTransFunc_sRGB)||
        (s.range!=MFNominalRange_0_255&&s.range!=MFNominalRange_16_235))throw Failure{MF_E_INVALIDMEDIATYPE,"Unsupported camera color"};
    return s;
}
static ComPtr<IMFMediaType> media_type(const GUID& subtype,UINT32 w,UINT32 h,UINT32 fps,Signal s) {
    ComPtr<IMFMediaType> type;check(MFCreateMediaType(&type),"Video media type");
    type->SetGUID(MF_MT_MAJOR_TYPE,MFMediaType_Video);type->SetGUID(MF_MT_SUBTYPE,subtype);
    MFSetAttributeSize(type.Get(),MF_MT_FRAME_SIZE,w,h);MFSetAttributeRatio(type.Get(),MF_MT_FRAME_RATE,fps,1);
    MFSetAttributeRatio(type.Get(),MF_MT_PIXEL_ASPECT_RATIO,1,1);type->SetUINT32(MF_MT_INTERLACE_MODE,MFVideoInterlace_Progressive);
    type->SetUINT32(MF_MT_YUV_MATRIX,s.matrix);type->SetUINT32(MF_MT_VIDEO_PRIMARIES,s.primaries);
    type->SetUINT32(MF_MT_TRANSFER_FUNCTION,s.transfer);type->SetUINT32(MF_MT_VIDEO_NOMINAL_RANGE,s.range);return type;
}
class Encoder {
    ComPtr<IMFActivate> activation;ComPtr<IMFTransform> transform;ComPtr<IMFMediaEventGenerator> events;ComPtr<ICodecAPI> codec;
    DWORD input=0,output=0;bool asynchronous=false;unsigned credits=0;
    struct Stamp {LONGLONG pts;unsigned long long captured;Clock::time_point submitted;};
    std::deque<Stamp> pending;ComPtr<IMFSample> output_sample;ComPtr<IMFMediaBuffer> output_buffer;
public:
    Encoder(Device& device,UINT32 w,UINT32 h,UINT32 fps,UINT32 bitrate,Signal color) {
        Activations list;MFT_REGISTER_TYPE_INFO in={MFMediaType_Video,MFVideoFormat_NV12},out={MFMediaType_Video,MFVideoFormat_HEVC};
        check(MFTEnumEx(MFT_CATEGORY_VIDEO_ENCODER,MFT_ENUM_FLAG_HARDWARE|MFT_ENUM_FLAG_SORTANDFILTER,&in,&out,&list.items,&list.count),"Hardware HEVC enumeration");
        HRESULT last=MF_E_TOPO_CODEC_NOT_FOUND;
        for(UINT32 i=0;i<list.count;++i) {
            try {
                ComPtr<IMFTransform> candidate;check(list.items[i]->ActivateObject(IID_PPV_ARGS(&candidate)),"Hardware HEVC activation");
                ComPtr<IMFAttributes> attrs;check(candidate->GetAttributes(&attrs),"Hardware HEVC attributes");
                UINT32 aware=0;attrs->GetUINT32(MF_SA_D3D11_AWARE,&aware);if(!aware)throw Failure{MF_E_UNSUPPORTED_D3D_TYPE,"HEVC encoder is not D3D11-aware"};
                UINT32 async=0;attrs->GetUINT32(MF_TRANSFORM_ASYNC,&async);asynchronous=async!=0;
                if(asynchronous)check(attrs->SetUINT32(MF_TRANSFORM_ASYNC_UNLOCK,TRUE),"Async HEVC unlock");
                check(candidate->ProcessMessage(MFT_MESSAGE_SET_D3D_MANAGER,reinterpret_cast<ULONG_PTR>(device.manager.Get())),"HEVC DXGI manager");
                DWORD inputs=0,outputs=0;check(candidate->GetStreamCount(&inputs,&outputs),"HEVC streams");
                if(inputs!=1||outputs!=1)throw Failure{E_NOTIMPL,"Unexpected HEVC stream count"};
                HRESULT ids=candidate->GetStreamIDs(1,&input,1,&output);if(ids==E_NOTIMPL)input=output=0;else check(ids,"HEVC stream identifiers");
                candidate.As(&codec);
                if(codec) {VARIANT v;VariantInit(&v);v.vt=VT_BOOL;v.boolVal=VARIANT_TRUE;codec->SetValue(&CODECAPI_AVLowLatencyMode,&v);
                    v.vt=VT_UI4;v.ulVal=0;codec->SetValue(&CODECAPI_AVEncMPVDefaultBPictureCount,&v);
                    v.ulVal=fps;codec->SetValue(&CODECAPI_AVEncMPVGOPSize,&v);}
                auto encoded=media_type(MFVideoFormat_HEVC,w,h,fps,color);encoded->SetUINT32(MF_MT_AVG_BITRATE,bitrate);
                check(candidate->SetOutputType(output,encoded.Get(),0),"HEVC output type");
                auto raw=media_type(MFVideoFormat_NV12,w,h,fps,color);check(candidate->SetInputType(input,raw.Get(),0),"Native NV12 HEVC input");
                if(asynchronous)check(candidate.As(&events),"Async HEVC events");
                transform=candidate;activation=list.items[i];
                check(transform->ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING,0),"HEVC begin");
                check(transform->ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM,0),"HEVC start");return;
            } catch(Failure f) {last=f.hr;codec.Reset();events.Reset();list.items[i]->ShutdownObject();}
        }throw Failure{last,"No usable D3D11 hardware HEVC encoder"};
    }
    ~Encoder(){if(transform){transform->ProcessMessage(MFT_MESSAGE_COMMAND_FLUSH,0);transform->ProcessMessage(MFT_MESSAGE_NOTIFY_END_STREAMING,0);}if(activation)activation->ShutdownObject();}
    bool ready()const{return pending.size()<4&&(!asynchronous||credits>0);}
    size_t inflight()const{return pending.size();}
    void force_keyframe(){if(codec){VARIANT v;VariantInit(&v);v.vt=VT_UI4;v.ulVal=1;check(codec->SetValue(&CODECAPI_AVEncVideoForceKeyFrame,&v),"HEVC keyframe request");}}
    void submit(Raw raw) {
        ComPtr<IMFMediaBuffer> buffer;check(raw.sample->GetBufferByIndex(0,&buffer),"Camera sample buffer");
        ComPtr<IMFDXGIBuffer> dxgi;check(buffer.As(&dxgi),"Camera did not supply a GPU surface");
        ComPtr<ID3D11Texture2D> texture;check(dxgi->GetResource(IID_PPV_ARGS(&texture)),"Camera D3D11 texture");
        D3D11_TEXTURE2D_DESC desc;texture->GetDesc(&desc);if(desc.Format!=DXGI_FORMAT_NV12)throw Failure{MF_E_INVALIDMEDIATYPE,"Camera GPU surface is not NV12"};
        LONGLONG pts;check(raw.sample->GetSampleTime(&pts),"Camera sample timestamp");
        auto submitted=Clock::now();check(transform->ProcessInput(input,raw.sample.Get(),0),"Hardware HEVC input");
        if(asynchronous)--credits;pending.push_back({pts,raw.captured,submitted});
    }
    void output_packet(Packet callback,void* context) {
        MFT_OUTPUT_STREAM_INFO info{};check(transform->GetOutputStreamInfo(output,&info),"HEVC output allocation");
        ComPtr<IMFSample> supplied;
        if(!(info.dwFlags&MFT_OUTPUT_STREAM_PROVIDES_SAMPLES)) {
            if(!output_sample) {
                check(MFCreateSample(&output_sample),"HEVC output sample");
                check(MFCreateMemoryBuffer(std::max<DWORD>(info.cbSize,16*1024*1024),&output_buffer),"HEVC compressed buffer");
                check(output_sample->AddBuffer(output_buffer.Get()),"HEVC output buffer");
            }
            output_buffer->SetCurrentLength(0);supplied=output_sample;
        }
        MFT_OUTPUT_DATA_BUFFER data{};data.dwStreamID=output;data.pSample=supplied.Get();DWORD status=0;
        HRESULT hr=transform->ProcessOutput(0,1,&data,&status);
        if(data.pEvents)data.pEvents->Release();
        ComPtr<IMFSample> result;if(data.pSample==supplied.Get())result=supplied;else result.Attach(data.pSample);
        if(hr==MF_E_TRANSFORM_NEED_MORE_INPUT&&!asynchronous)return;
        check(hr,"Hardware HEVC output");if(!result)throw Failure{E_UNEXPECTED,"Missing HEVC output sample"};
        LONGLONG pts;check(result->GetSampleTime(&pts),"HEVC output timestamp");
        auto stamp=std::find_if(pending.begin(),pending.end(),[pts](const Stamp& s){return s.pts==pts;});
        if(stamp==pending.end())throw Failure{E_UNEXPECTED,"HEVC frame timestamp crossed ownership"};
        ComPtr<IMFMediaBuffer> buffer;check(result->ConvertToContiguousBuffer(&buffer),"HEVC compressed bytes");
        BYTE* bytes=nullptr;DWORD length=0;check(buffer->Lock(&bytes,nullptr,&length),"HEVC compressed output");
        // Only the compressed access unit is locked; captured pixels stay on D3D11.
        if(!length||length>16*1024*1024){buffer->Unlock();throw Failure{E_BOUNDS,"Invalid HEVC output length"};}
        callback(context,bytes,length,stamp->captured,
            std::chrono::duration_cast<std::chrono::microseconds>(Clock::now()-stamp->submitted).count());
        buffer->Unlock();pending.erase(stamp);
    }
    void drain(Packet callback,void* context) {
        if(!asynchronous){if(!pending.empty())output_packet(callback,context);return;}
        for(unsigned n=0;n<64;++n) {
            ComPtr<IMFMediaEvent> event;HRESULT hr=events->GetEvent(MF_EVENT_FLAG_NO_WAIT,&event);
            if(hr==MF_E_NO_EVENTS_AVAILABLE)return;check(hr,"HEVC event");HRESULT status;check(event->GetStatus(&status),"HEVC event status");check(status,"HEVC async operation");
            MediaEventType type;check(event->GetType(&type),"HEVC event type");
            if(type==METransformNeedInput){if(++credits>64)throw Failure{E_UNEXPECTED,"Unbounded HEVC input credits"};}
            else if(type==METransformHaveOutput)output_packet(callback,context);
        }
    }
};
struct Capture {
    std::shared_ptr<Shared> state;std::thread worker;
    Capture(const char* key,UINT32 width,UINT32 height,UINT32 fps,UINT32 bitrate,void* context,Timestamp timestamp,Admit admit,Format configured,Packet packet,Error error)
        :state(std::make_shared<Shared>(context,timestamp)),worker([=,key=std::string(key),s=state]{
        try {
            Apartment apartment;MediaFoundation mf;Device device;
            ComPtr<IMFAttributes> attrs;check(MFCreateAttributes(&attrs,2),"Camera source attributes");
            attrs->SetGUID(MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE,MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_GUID);
            auto path=wide(key.c_str());attrs->SetString(MF_DEVSOURCE_ATTRIBUTE_SOURCE_TYPE_VIDCAP_SYMBOLIC_LINK,path.c_str());
            ComPtr<IMFMediaSource> source;check(MFCreateDeviceSource(attrs.Get(),&source),"Open selected camera; check privacy permission or occupancy");
            struct Shutdown {IMFMediaSource* source;~Shutdown(){source->Shutdown();}} shutdown{source.Get()};
            auto callback=Make<ReaderCallback>(s);ComPtr<IMFAttributes> options;MFCreateAttributes(&options,6);
            options->SetUnknown(MF_SOURCE_READER_ASYNC_CALLBACK,static_cast<IMFSourceReaderCallback*>(callback.Get()));options->SetUnknown(MF_SOURCE_READER_D3D_MANAGER,device.manager.Get());
            options->SetUINT32(MF_READWRITE_D3D_OPTIONAL,FALSE);options->SetUINT32(MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS,TRUE);
            options->SetUINT32(MF_READWRITE_DISABLE_CONVERTERS,TRUE);options->SetUINT32(MF_LOW_LATENCY,TRUE);
            ComPtr<IMFSourceReader> reader;check(MFCreateSourceReaderFromMediaSource(source.Get(),options.Get(),&reader),"Native camera Source Reader");
            ComPtr<IMFMediaType> selected;UINT32 best_width=0,best_height=0;UINT64 best_area=0;
            for(DWORD i=0;i<512;++i) {
                ComPtr<IMFMediaType> type;HRESULT hr=reader->GetNativeMediaType(MF_SOURCE_READER_FIRST_VIDEO_STREAM,i,&type);
                if(hr==MF_E_NO_MORE_TYPES)break;check(hr,"Camera native mode");UINT32 w=0,h=0;GUID subtype;
                if(FAILED(MFGetAttributeSize(type.Get(),MF_MT_FRAME_SIZE,&w,&h))||FAILED(type->GetGUID(MF_MT_SUBTYPE,&subtype)))continue;
                UINT64 area=UINT64(w)*h;if(subtype!=MFVideoFormat_NV12||w<2||h<2||w%2||h%2||w>width||h>height||area<=best_area)continue;
                selected=type;best_width=w;best_height=h;best_area=area;
            }
            if(!selected)throw Failure{MF_E_INVALIDMEDIATYPE,"Camera has no native NV12 mode within requested size"};
            check(reader->SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS,FALSE),"Disable non-video camera streams");
            check(reader->SetStreamSelection(MF_SOURCE_READER_FIRST_VIDEO_STREAM,TRUE),"Select camera video stream");
            check(reader->SetCurrentMediaType(MF_SOURCE_READER_FIRST_VIDEO_STREAM,nullptr,selected.Get()),"Select native camera mode");
            ComPtr<IMFMediaType> actual;check(reader->GetCurrentMediaType(MF_SOURCE_READER_FIRST_VIDEO_STREAM,&actual),"Actual camera media type");
            auto color=signal(actual.Get());UINT32 numerator=0,denominator=0;check(MFGetAttributeRatio(actual.Get(),MF_MT_FRAME_RATE,&numerator,&denominator),"Camera frame rate");
            configured(context,best_width,best_height,color.matrix==MFVideoTransferMatrix_BT709?1:6,1,
                color.transfer==MFVideoTransFunc_sRGB?13:1,color.range==MFNominalRange_0_255);
            if(!denominator||numerator>120*denominator)throw Failure{MF_E_INVALIDMEDIATYPE,"Unsupported camera frame rate"};
            UINT32 rate=std::max<UINT32>(1,std::min(fps,numerator/denominator));Encoder encoder(device,best_width,best_height,rate,bitrate,color);
            while(!s->stop) {
                encoder.drain(packet,context);
                if(s->keyframe.exchange(false))encoder.force_keyframe();
                Raw raw;bool read=false;
                {std::unique_lock<std::mutex> lock(s->mutex);check(s->failure,"Camera read stopped or permission changed");
                    if(encoder.ready()&&s->latest.sample)raw=std::move(s->latest);
                    if(!s->read_pending){s->read_pending=true;read=true;}
                }
                if(read)check(reader->ReadSample(MF_SOURCE_READER_FIRST_VIDEO_STREAM,0,nullptr,nullptr,nullptr,nullptr),"Camera asynchronous read");
                if(raw.sample) {
                    LONGLONG source_time=0;check(raw.sample->GetSampleTime(&source_time),"Camera source pacing timestamp");
                    if(source_time<0)throw Failure{MF_E_INVALID_TIMESTAMP,"Negative camera source timestamp"};
                    if(admit(context,static_cast<unsigned long long>(source_time)/10))encoder.submit(std::move(raw));
                }
                std::unique_lock<std::mutex> lock(s->mutex);
                s->wake.wait_for(lock,std::chrono::milliseconds(encoder.inflight()?2:16),[&]{return s->stop||FAILED(s->failure)||(!s->read_pending&&encoder.ready());});
            }
            reader->Flush(MF_SOURCE_READER_ALL_STREAMS);reader.Reset();
            {std::lock_guard<std::mutex> lock(s->mutex);s->latest={};}
        } catch(Failure f){if(!s->stop)report(error,context,f);}catch(...){if(!s->stop)error(context,"Windows native camera worker failed");}
    }){}
};
extern "C" void* weavelo_windows_camera_start(const char* key,UINT32 w,UINT32 h,UINT32 fps,UINT32 bitrate,void* context,Timestamp timestamp,Admit admit,Format configured,Packet packet,Error error) {
    try {if(!key||w<2||h<2||fps<1||fps>120||!bitrate)throw Failure{E_INVALIDARG,"Invalid camera request"};
        return new Capture(key,w,h,fps,bitrate,context,timestamp,admit,configured,packet,error);
    }catch(Failure f){report(error,context,f);}catch(...){error(context,"Cannot start native camera worker");}return nullptr;
}
extern "C" void weavelo_windows_camera_keyframe(void* handle){static_cast<Capture*>(handle)->state->keyframe=true;}
// Called on a Rust teardown worker. The completion is acknowledged only after
// Source Reader callbacks, GPU encoder, camera and COM apartment have ended.
extern "C" void weavelo_windows_camera_stop(void* handle) {
    auto camera=static_cast<Capture*>(handle);camera->state->stop=true;camera->state->wake.notify_all();
    camera->worker.join();delete camera;
}

// Synthetic GPU surfaces exercise exactly the production hardware encoder,
// independently of camera presence/consent. Never used as a product source.
extern "C" int weavelo_windows_camera_test_gpu(UINT32 variant,void* context,Packet packet,Error error) {
    try {
        Apartment apartment;MediaFoundation mf;Device device;
        if(variant>7)throw Failure{E_INVALIDARG,"Invalid GPU color fixture"};
        Signal color{variant&1?MFVideoTransferMatrix_BT601:MFVideoTransferMatrix_BT709,MFVideoPrimaries_BT709,
            variant&2?MFVideoTransFunc_sRGB:MFVideoTransFunc_709,variant&4?MFNominalRange_0_255:MFNominalRange_16_235};
        Encoder encoder(device,320,180,30,2000000,color);unsigned submitted=0;
        auto deadline=Clock::now()+std::chrono::seconds(12);
        while(submitted<36||encoder.inflight()) {
            if(Clock::now()>deadline)throw Failure{HRESULT_FROM_WIN32(WAIT_TIMEOUT),"Hardware HEVC fixture timeout"};
            encoder.drain(packet,context);
            if(submitted<36&&encoder.ready()) {
                if(submitted==20)encoder.force_keyframe();
                std::vector<BYTE> bytes(320*180*3/2,128);
                for(unsigned y=0;y<180;++y)for(unsigned x=0;x<320;++x)bytes[y*320+x]=BYTE(40+(x/80)*50+submitted%2);
                D3D11_TEXTURE2D_DESC desc{};desc.Width=320;desc.Height=180;desc.MipLevels=1;desc.ArraySize=1;
                desc.Format=DXGI_FORMAT_NV12;desc.SampleDesc.Count=1;desc.Usage=D3D11_USAGE_DEFAULT;desc.BindFlags=D3D11_BIND_DECODER|D3D11_BIND_SHADER_RESOURCE;
                D3D11_SUBRESOURCE_DATA initial{bytes.data(),320,0};ComPtr<ID3D11Texture2D> texture;
                check(device.device->CreateTexture2D(&desc,&initial,&texture),"GPU fixture texture");
                ComPtr<IMFMediaBuffer> buffer;check(MFCreateDXGISurfaceBuffer(__uuidof(ID3D11Texture2D),texture.Get(),0,FALSE,&buffer),"GPU fixture MF buffer");
                ComPtr<IMFSample> sample;check(MFCreateSample(&sample),"GPU fixture sample");sample->AddBuffer(buffer.Get());
                sample->SetSampleTime(LONGLONG(submitted)*10000000/30);sample->SetSampleDuration(10000000/30);
                encoder.submit({sample,1000000+static_cast<unsigned long long>(submitted)*33333});++submitted;
            } else std::this_thread::sleep_for(std::chrono::milliseconds(1));
        }return 1;
    }catch(Failure f){report(error,context,f);}catch(...){error(context,"GPU HEVC fixture failed");}return 0;
}
