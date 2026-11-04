// Native camera capture; shares the original hardware codec with screen capture.
#include "windows_media.h"
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
static void report(NativeError error,void* context,Failure f) {
    char message[256];snprintf(message,sizeof(message),"Windows camera: %s (0x%08lx)",f.stage,static_cast<unsigned long>(f.hr));
    error(context,message);
}
extern "C" int weavelo_windows_camera_catalog(void* context,void(*item)(void*,const char*,const char*),NativeError error) {
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
struct Capture {
    std::shared_ptr<Shared> state;std::thread worker;
    Capture(const char* key,UINT32 width,UINT32 height,UINT32 fps,UINT32 bitrate,void* context,Timestamp timestamp,Admit admit,Format configured,Packet packet,NativeError error)
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
extern "C" void* weavelo_windows_camera_start(const char* key,UINT32 w,UINT32 h,UINT32 fps,UINT32 bitrate,void* context,Timestamp timestamp,Admit admit,Format configured,Packet packet,NativeError error) {
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
extern "C" int weavelo_windows_camera_test_gpu(UINT32 variant,void* context,Packet packet,NativeError error) {
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
