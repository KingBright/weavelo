// Shared native D3D11 / Media Foundation hardware codec for screen and camera.
#pragma once
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
#include <array>
using Microsoft::WRL::ComPtr;
using Microsoft::WRL::RuntimeClass;
using Microsoft::WRL::RuntimeClassFlags;
using Microsoft::WRL::ClassicCom;
using Microsoft::WRL::FtmBase;
using Microsoft::WRL::Make;
using Clock=std::chrono::steady_clock;
using Packet=void(*)(void*,const unsigned char*,size_t,unsigned long long,unsigned long long);
using NativeError=void(*)(void*,const char*);
using Timestamp=unsigned long long(*)(void*);
using Admit=bool(*)(void*,unsigned long long);
using Format=void(*)(void*,UINT32,UINT32,UINT32,UINT32,UINT32,bool);
using DetailedPacket=void(*)(void*,const unsigned char*,size_t,unsigned long long,unsigned long long,const unsigned char*,size_t);
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
struct Raw {
    ComPtr<IMFSample> sample;unsigned long long captured=0;
    std::shared_ptr<void> lease;
    std::array<unsigned char,60> geometry{};
    bool has_geometry=false;
    Clock::time_point submitted{};
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
    struct Stamp {LONGLONG pts;unsigned long long captured;Clock::time_point submitted;std::shared_ptr<void> lease;std::array<unsigned char,60> geometry;bool has_geometry;};
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
    void finish_input(){check(transform->ProcessMessage(MFT_MESSAGE_NOTIFY_END_OF_STREAM,0),"HEVC end of input");check(transform->ProcessMessage(MFT_MESSAGE_COMMAND_DRAIN,0),"HEVC drain");}
    void force_keyframe(){if(codec){VARIANT v;VariantInit(&v);v.vt=VT_UI4;v.ulVal=1;check(codec->SetValue(&CODECAPI_AVEncVideoForceKeyFrame,&v),"HEVC keyframe request");}}
    void submit(Raw raw) {
        ComPtr<IMFMediaBuffer> buffer;check(raw.sample->GetBufferByIndex(0,&buffer),"Camera sample buffer");
        ComPtr<IMFDXGIBuffer> dxgi;check(buffer.As(&dxgi),"Camera did not supply a GPU surface");
        ComPtr<ID3D11Texture2D> texture;check(dxgi->GetResource(IID_PPV_ARGS(&texture)),"Camera D3D11 texture");
        D3D11_TEXTURE2D_DESC desc;texture->GetDesc(&desc);if(desc.Format!=DXGI_FORMAT_NV12)throw Failure{MF_E_INVALIDMEDIATYPE,"Camera GPU surface is not NV12"};
        LONGLONG pts;check(raw.sample->GetSampleTime(&pts),"Camera sample timestamp");
        auto submitted=raw.submitted==Clock::time_point{}?Clock::now():raw.submitted;check(transform->ProcessInput(input,raw.sample.Get(),0),"Hardware HEVC input");
        if(asynchronous)--credits;pending.push_back({pts,raw.captured,submitted,std::move(raw.lease),raw.geometry,raw.has_geometry});
    }
    void output_packet(Packet callback,void* context,DetailedPacket detailed=nullptr) {
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
        const auto cost=std::chrono::duration_cast<std::chrono::microseconds>(Clock::now()-stamp->submitted).count();
        if(detailed) detailed(context,bytes,length,stamp->captured,cost,
            stamp->has_geometry?stamp->geometry.data():nullptr,stamp->has_geometry?stamp->geometry.size():0);
        else callback(context,bytes,length,stamp->captured,cost);
        buffer->Unlock();pending.erase(stamp);
    }
    void drain(Packet callback,void* context,DetailedPacket detailed=nullptr) {
        if(!asynchronous){if(!pending.empty())output_packet(callback,context,detailed);return;}
        for(unsigned n=0;n<64;++n) {
            ComPtr<IMFMediaEvent> event;HRESULT hr=events->GetEvent(MF_EVENT_FLAG_NO_WAIT,&event);
            if(hr==MF_E_NO_EVENTS_AVAILABLE)return;check(hr,"HEVC event");HRESULT status;check(event->GetStatus(&status),"HEVC event status");check(status,"HEVC async operation");
            MediaEventType type;check(event->GetType(&type),"HEVC event type");
            if(type==METransformNeedInput){if(++credits>64)throw Failure{E_UNEXPECTED,"Unbounded HEVC input credits"};}
            else if(type==METransformHaveOutput)output_packet(callback,context,detailed);
        }
    }
};
