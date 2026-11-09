// One WGC capture per authorized source, independent native GPU views/codecs.
#include "windows_media.h"
#include <dwmapi.h>
#include <dxgi1_6.h>
#include <d3d11_1.h>
#include <windows.graphics.capture.interop.h>
#include <windows.graphics.directx.direct3d11.interop.h>
#include <winrt/Windows.Foundation.h>
#include <winrt/Windows.Graphics.Capture.h>
#include <winrt/Windows.Graphics.DirectX.h>
#include <winrt/Windows.Graphics.DirectX.Direct3D11.h>
#include <map>
#include <tuple>
#include <cstring>
namespace {
namespace Capture=winrt::Windows::Graphics::Capture;
namespace DirectX=winrt::Windows::Graphics::DirectX;
namespace Direct3D=winrt::Windows::Graphics::DirectX::Direct3D11;
struct CaptureApartment {
    CaptureApartment() { winrt::init_apartment(winrt::apartment_type::multi_threaded); }
    ~CaptureApartment() { winrt::uninit_apartment(); }
};
struct Plan {
    UINT32 x,y,crop_width,crop_height,width,height,fps,kbps;
    std::array<unsigned char,60> geometry;
};
static_assert(sizeof(Plan)==92,"Rust/C++ viewport plan layout");
using MakePlan=bool(*)(void*,UINT32,UINT32,Plan*);
using ScreenTimestamp=unsigned long long(*)();
struct RawFrame {
    Capture::Direct3D11CaptureFrame frame{nullptr};ComPtr<ID3D11Texture2D> texture;
    unsigned long long captured=0,sequence=0;UINT32 width=0,height=0;
    ~RawFrame(){try{if(frame)frame.Close();}catch(...){} }
};
struct Root:std::enable_shared_from_this<Root> {
    Device gpu;std::mutex state,gpu_use,receive_use;std::condition_variable changed;
    Capture::GraphicsCaptureItem item{nullptr};Capture::Direct3D11CaptureFramePool pool{nullptr};
    Capture::GraphicsCaptureSession session{nullptr};Direct3D::IDirect3DDevice device{nullptr};
    winrt::event_token arrived{},closed{};bool listening=false;UINT32 pool_width=0,pool_height=0;
    std::shared_ptr<RawFrame> latest;HRESULT failure=S_OK;unsigned long long sequence=0;
    ScreenTimestamp timestamp;
    explicit Root(ScreenTimestamp clock):timestamp(clock){}
    void fail(HRESULT hr){std::lock_guard<std::mutex> lock(state);failure=hr;changed.notify_all();}
    void initialize(UINT64 handle,bool window) {
        // Capture workers may tear down the last WinRT apartment before a
        // later source starts. Keep activation factories local to this call.
        auto support=winrt::get_activation_factory<Capture::GraphicsCaptureSession,Capture::IGraphicsCaptureSessionStatics>();
        if(!support.IsSupported())throw Failure{E_NOTIMPL,"Windows Graphics Capture is unavailable"};
        HMONITOR monitor=window?MonitorFromWindow(reinterpret_cast<HWND>(handle),MONITOR_DEFAULTTONEAREST):reinterpret_cast<HMONITOR>(handle);
        ComPtr<IDXGIDevice> dxgi;check(gpu.device.As(&dxgi),"Capture DXGI device");
        ComPtr<IDXGIAdapter> adapter;check(dxgi->GetAdapter(&adapter),"Capture adapter");
        bool color_known=false;
        for(UINT i=0;;++i){ComPtr<IDXGIOutput> output;HRESULT hr=adapter->EnumOutputs(i,&output);if(hr==DXGI_ERROR_NOT_FOUND)break;check(hr,"Capture outputs");
            DXGI_OUTPUT_DESC desc{};check(output->GetDesc(&desc),"Capture monitor");if(desc.Monitor!=monitor)continue;
            ComPtr<IDXGIOutput6> output6;check(output.As(&output6),"Capture monitor color metadata");DXGI_OUTPUT_DESC1 actual{};
            check(output6->GetDesc1(&actual),"Capture monitor color space");
            if(actual.ColorSpace!=DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709)throw Failure{E_NOTIMPL,"HDR capture requires its native color path"};
            color_known=true;break;
        }
        if(!color_known)throw Failure{E_NOTIMPL,"Capture monitor is not on the active D3D11 adapter"};
        winrt::com_ptr<IInspectable> inspectable;
        check(CreateDirect3D11DeviceFromDXGIDevice(dxgi.Get(),inspectable.put()),"WGC Direct3D device");
        device=inspectable.as<Direct3D::IDirect3DDevice>();
        auto interop=winrt::get_activation_factory<Capture::GraphicsCaptureItem,IGraphicsCaptureItemInterop>();
        if(window)check(interop->CreateForWindow(reinterpret_cast<HWND>(handle),winrt::guid_of<Capture::GraphicsCaptureItem>(),winrt::put_abi(item)),"WGC window");
        else check(interop->CreateForMonitor(monitor,winrt::guid_of<Capture::GraphicsCaptureItem>(),winrt::put_abi(item)),"WGC monitor");
        auto size=item.Size();if(size.Width<2||size.Height<2)throw Failure{E_BOUNDS,"Empty WGC source"};
        pool_width=size.Width;pool_height=size.Height;
        auto pool_factory=winrt::get_activation_factory<Capture::Direct3D11CaptureFramePool,Capture::IDirect3D11CaptureFramePoolStatics2>();
        pool=pool_factory.CreateFreeThreaded(device,DirectX::DirectXPixelFormat::B8G8R8A8UIntNormalized,3,size);
        session=pool.CreateCaptureSession(item);
        std::weak_ptr<Root> weak=shared_from_this();
        closed=item.Closed([weak](auto const&,auto const&){if(auto root=weak.lock())root->fail(RO_E_CLOSED);});
        arrived=pool.FrameArrived([weak](auto const&,auto const&){if(auto root=weak.lock())root->receive();});
        listening=true;session.StartCapture();
    }
    void receive() noexcept {
        try {
            std::lock_guard<std::mutex> serial(receive_use);
            auto frame=pool.TryGetNextFrame();if(!frame)return;
            const auto captured=timestamp();const auto size=frame.ContentSize();
            if(size.Width<2||size.Height<2){frame.Close();return;}
            if(UINT32(size.Width)!=pool_width||UINT32(size.Height)!=pool_height){
                {std::lock_guard<std::mutex> lock(state);latest.reset();}
                frame.Close();pool.Recreate(device,DirectX::DirectXPixelFormat::B8G8R8A8UIntNormalized,3,size);
                pool_width=size.Width;pool_height=size.Height;return;
            }
            auto raw=std::make_shared<RawFrame>();raw->frame=std::move(frame);raw->captured=captured;raw->width=size.Width;raw->height=size.Height;
            auto access=raw->frame.Surface().as<::Windows::Graphics::DirectX::Direct3D11::IDirect3DDxgiInterfaceAccess>();check(access->GetInterface(IID_PPV_ARGS(&raw->texture)),"WGC native texture");
            std::lock_guard<std::mutex> lock(state);raw->sequence=++sequence;latest=std::move(raw);changed.notify_all();
        }catch(winrt::hresult_error const& e){fail(e.code());}catch(Failure f){fail(f.hr);}catch(...){fail(E_FAIL);}
    }
    ~Root(){
        try{if(listening)pool.FrameArrived(arrived);}catch(...){}
        try{if(listening)item.Closed(closed);}catch(...){}
        try{if(session)session.Close();}catch(...){}
        try{if(pool)pool.Close();}catch(...){}
        latest.reset();
    }
};
using RootKey=std::tuple<UINT64,UINT32,bool,bool,std::array<unsigned char,32>>;
std::mutex roots_mutex;std::map<RootKey,std::weak_ptr<Root>> roots;
static std::shared_ptr<Root> capture_root(UINT64 handle,UINT32 source,bool window,bool scoped,const unsigned char* scope,ScreenTimestamp timestamp) {
    std::array<unsigned char,32> network{};if(scoped)std::copy_n(scope,network.size(),network.begin());
    RootKey key{handle,source,window,scoped,network};std::lock_guard<std::mutex> lock(roots_mutex);
    for(auto it=roots.begin();it!=roots.end();){if(it->second.expired())it=roots.erase(it);else ++it;}
    if(auto root=roots[key].lock())return root;
    auto root=std::make_shared<Root>(timestamp);root->initialize(handle,window);roots[key]=root;return root;
}
struct Slot {ComPtr<ID3D11Texture2D> texture;ComPtr<ID3D11VideoProcessorOutputView> output;bool used=false;};
struct SlotLease {std::shared_ptr<Slot> slot;explicit SlotLease(std::shared_ptr<Slot> value):slot(std::move(value)){slot->used=true;}~SlotLease(){slot->used=false;}};
class Cropper {
    Root& root;ComPtr<ID3D11VideoDevice> device;ComPtr<ID3D11VideoContext1> context;
    ComPtr<ID3D11VideoProcessorEnumerator> enumerator;ComPtr<ID3D11VideoProcessor> processor;
    std::vector<std::shared_ptr<Slot>> slots;
public:
    Cropper(Root& root,UINT32 source_width,UINT32 source_height,const Plan& p):root(root) {
        check(root.gpu.device.As(&device),"D3D11 video processor device");check(root.gpu.context.As(&context),"D3D11 video color context");
        D3D11_VIDEO_PROCESSOR_CONTENT_DESC desc{};desc.InputFrameFormat=D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE;
        desc.InputWidth=source_width;desc.InputHeight=source_height;desc.OutputWidth=p.width;desc.OutputHeight=p.height;
        desc.InputFrameRate={p.fps,1};desc.OutputFrameRate={p.fps,1};desc.Usage=D3D11_VIDEO_USAGE_PLAYBACK_NORMAL;
        check(device->CreateVideoProcessorEnumerator(&desc,&enumerator),"GPU viewport enumerator");
        check(device->CreateVideoProcessor(enumerator.Get(),0,&processor),"GPU viewport processor");
        for(unsigned i=0;i<4;++i){auto slot=std::make_shared<Slot>();D3D11_TEXTURE2D_DESC texture{};
            texture.Width=p.width;texture.Height=p.height;texture.MipLevels=1;texture.ArraySize=1;texture.Format=DXGI_FORMAT_NV12;
            texture.SampleDesc.Count=1;texture.Usage=D3D11_USAGE_DEFAULT;texture.BindFlags=D3D11_BIND_RENDER_TARGET|D3D11_BIND_VIDEO_ENCODER;
            check(root.gpu.device->CreateTexture2D(&texture,nullptr,&slot->texture),"NV12 viewport pool");
            D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC view{};view.ViewDimension=D3D11_VPOV_DIMENSION_TEXTURE2D;
            check(device->CreateVideoProcessorOutputView(slot->texture.Get(),enumerator.Get(),&view,&slot->output),"NV12 viewport output");slots.push_back(slot);
        }
    }
    bool available()const{return std::any_of(slots.begin(),slots.end(),[](const auto& s){return !s->used;});}
    Raw crop(const std::shared_ptr<RawFrame>& frame,RECT base,const Plan& p,LONGLONG pts) {
        const auto submitted=Clock::now();
        auto slot=*std::find_if(slots.begin(),slots.end(),[](const auto& s){return !s->used;});
        auto lease=std::make_shared<SlotLease>(slot);
        ComPtr<ID3D11VideoProcessorInputView> input;D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC view{};view.ViewDimension=D3D11_VPIV_DIMENSION_TEXTURE2D;
        check(device->CreateVideoProcessorInputView(frame->texture.Get(),enumerator.Get(),&view,&input),"WGC GPU input view");
        RECT source{base.left+LONG(p.x),base.top+LONG(p.y),base.left+LONG(p.x+p.crop_width),base.top+LONG(p.y+p.crop_height)};
        RECT output{0,0,LONG(p.width),LONG(p.height)};ComPtr<ID3D11Query> completion;D3D11_QUERY_DESC query{D3D11_QUERY_EVENT,0};
        check(root.gpu.device->CreateQuery(&query,&completion),"GPU capture completion");
        {
            std::lock_guard<std::mutex> lock(root.gpu_use);
            context->VideoProcessorSetStreamFrameFormat(processor.Get(),0,D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE);
            context->VideoProcessorSetStreamSourceRect(processor.Get(),0,TRUE,&source);
            context->VideoProcessorSetStreamDestRect(processor.Get(),0,TRUE,&output);
            context->VideoProcessorSetOutputTargetRect(processor.Get(),TRUE,&output);
            context->VideoProcessorSetStreamColorSpace1(processor.Get(),0,DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709);
            context->VideoProcessorSetOutputColorSpace1(processor.Get(),DXGI_COLOR_SPACE_YCBCR_STUDIO_G22_LEFT_P709);
            context->VideoProcessorSetStreamAutoProcessingMode(processor.Get(),0,FALSE);
            D3D11_VIDEO_PROCESSOR_STREAM stream{};stream.Enable=TRUE;stream.pInputSurface=input.Get();
            check(context->VideoProcessorBlt(processor.Get(),slot->output.Get(),0,1,&stream),"Native GPU viewport crop");
            root.gpu.context->End(completion.Get());root.gpu.context->Flush();
        }
        // WGC owns its source buffer until the GPU has finished reading it.
        // Flush only submits work; an event query proves its actual completion.
        for(;;){HRESULT hr=root.gpu.context->GetData(completion.Get(),nullptr,0,0);if(hr==S_OK)break;check(hr,"GPU viewport completion");std::this_thread::yield();}
        Raw raw;raw.captured=frame->captured;raw.lease=std::move(lease);raw.geometry=p.geometry;raw.has_geometry=true;raw.submitted=submitted;
        check(MFCreateSample(&raw.sample),"Viewport sample");ComPtr<IMFMediaBuffer> buffer;
        check(MFCreateDXGISurfaceBuffer(__uuidof(ID3D11Texture2D),slot->texture.Get(),0,FALSE,&buffer),"Viewport GPU buffer");
        check(raw.sample->AddBuffer(buffer.Get()),"Viewport sample buffer");check(raw.sample->SetSampleTime(pts),"Viewport PTS");
        check(raw.sample->SetSampleDuration(10'000'000/p.fps),"Viewport frame duration");return raw;
    }
};
static RECT source_rect(UINT64 handle,bool window,const RawFrame& frame) {
    if(!window)return RECT{0,0,LONG(frame.width),LONG(frame.height)};
    HWND hwnd=reinterpret_cast<HWND>(handle);RECT client{},outer{};POINT origin{};
    check(GetClientRect(hwnd,&client)?S_OK:HRESULT_FROM_WIN32(GetLastError()),"Capture client bounds");
    check(ClientToScreen(hwnd,&origin)?S_OK:HRESULT_FROM_WIN32(GetLastError()),"Capture client position");
    check(DwmGetWindowAttribute(hwnd,DWMWA_EXTENDED_FRAME_BOUNDS,&outer,sizeof(outer)),"WGC window bounds");
    // A resized source can arrive before its old pool buffer is retired. Wait
    // for a matching physical allocation instead of shifting input coordinates.
    if(outer.right-outer.left!=LONG(frame.width)||outer.bottom-outer.top!=LONG(frame.height))return RECT{};
    RECT rect{origin.x-outer.left,origin.y-outer.top,origin.x-outer.left+client.right,origin.y-outer.top+client.bottom};
    if(rect.left<0||rect.top<0||rect.right>LONG(frame.width)||rect.bottom>LONG(frame.height))return RECT{};return rect;
}
struct Worker {
    std::atomic<bool> stop{false},keyframe{false};std::thread thread;std::weak_ptr<Root> root;
    std::mutex state;
    void wake(){std::lock_guard<std::mutex> lock(state);if(auto value=root.lock())value->changed.notify_all();}
};
static void run(Worker* owner,UINT64 handle,UINT32 source,bool window,bool scoped,std::array<unsigned char,32> scope,
    void* context,ScreenTimestamp timestamp,Admit admit,MakePlan make_plan,DetailedPacket packet,NativeError error) noexcept {
    try {
        CaptureApartment apartment;MediaFoundation foundation;
        // Coordinates are physical pixels on this native worker too.
        auto previous=SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        struct Dpi {DPI_AWARENESS_CONTEXT old;~Dpi(){if(old)SetThreadDpiAwarenessContext(old);}} dpi{previous};
        auto root=capture_root(handle,source,window,scoped,scope.data(),timestamp);{std::lock_guard<std::mutex> lock(owner->state);owner->root=root;}
        std::unique_ptr<Encoder> encoder;std::unique_ptr<Cropper> cropper;Plan active{},last{};
        UINT32 raw_width=0,raw_height=0;unsigned long long submitted_sequence=0;LONGLONG pts=0;bool draining=false;
        const Signal color{MFVideoTransferMatrix_BT709,MFVideoPrimaries_BT709,MFVideoTransFunc_sRGB,MFNominalRange_16_235};
        while(!owner->stop.load()) {
            if(encoder)encoder->drain(nullptr,context,packet);
            std::shared_ptr<RawFrame> frame;
            {std::lock_guard<std::mutex> lock(root->state);check(root->failure,"WGC capture ended");frame=root->latest;}
            if(frame){
                RECT rect=source_rect(handle,window,*frame);Plan plan{};
                UINT32 w=rect.right-rect.left,h=rect.bottom-rect.top;
                if(w>=2&&h>=2&&make_plan(context,w,h,&plan)) {
                    if(plan.x+plan.crop_width>w||plan.y+plan.crop_height>h||plan.width<2||plan.height<2||!plan.fps||!plan.kbps)
                        throw Failure{E_BOUNDS,"Invalid viewport geometry"};
                    const bool changed=draining||!encoder||active.width!=plan.width||active.height!=plan.height||active.fps!=plan.fps||active.kbps!=plan.kbps;
                    if(changed&&encoder&&encoder->inflight()) {if(!draining){encoder->finish_input();draining=true;}}
                    else {
                        if(changed){encoder.reset();cropper.reset();encoder=std::make_unique<Encoder>(root->gpu,plan.width,plan.height,plan.fps,plan.kbps*1000,color);active=plan;draining=false;}
                        if(!cropper||changed||raw_width!=frame->width||raw_height!=frame->height){cropper=std::make_unique<Cropper>(*root,frame->width,frame->height,plan);raw_width=frame->width;raw_height=frame->height;}
                        const bool refresh=changed||owner->keyframe.load()||frame->sequence!=submitted_sequence||std::memcmp(&plan,&last,4*sizeof(UINT32))!=0;
                        if(refresh&&encoder->ready()&&cropper->available()&&admit(context,timestamp())) {
                            if(owner->keyframe.exchange(false))encoder->force_keyframe();
                            auto raw=cropper->crop(frame,rect,plan,++pts*10'000'000/plan.fps);
                            encoder->submit(std::move(raw));last=plan;submitted_sequence=frame->sequence;
                        }
                    }
                }
            }
            frame.reset();std::unique_lock<std::mutex> lock(root->state);
            root->changed.wait_for(lock,encoder&&encoder->inflight()?std::chrono::milliseconds(1):std::chrono::milliseconds(10));
        }
    }catch(winrt::hresult_error const& e){char message[160];std::snprintf(message,sizeof(message),"WGC native operation failed (0x%08lx)",static_cast<unsigned long>(e.code().value));error(context,message);}
    catch(Failure f){char message[200];std::snprintf(message,sizeof(message),"%s (0x%08lx)",f.stage,static_cast<unsigned long>(f.hr));error(context,message);}
    catch(...){error(context,"Native WGC worker failed");}
}
}
extern "C" void* weavelo_windows_screen_start(UINT64 handle,UINT32 source,bool window,bool scoped,const unsigned char* scope,
    void* context,ScreenTimestamp timestamp,Admit admit,MakePlan plan,DetailedPacket packet,NativeError error) {
    try {if(!handle||!source||!timestamp||!admit||!plan||!packet||!error||(scoped&&!scope))return nullptr;
        auto owner=std::make_unique<Worker>();std::array<unsigned char,32> network{};if(scoped)std::copy_n(scope,network.size(),network.begin());
        owner->thread=std::thread(run,owner.get(),handle,source,window,scoped,network,context,timestamp,admit,plan,packet,error);return owner.release();
    }catch(...){return nullptr;}
}
extern "C" void weavelo_windows_screen_changed(void* handle,bool keyframe){auto owner=static_cast<Worker*>(handle);if(keyframe)owner->keyframe=true;owner->wake();}
extern "C" void weavelo_windows_screen_stop(void* handle){std::unique_ptr<Worker> owner(static_cast<Worker*>(handle));owner->stop=true;owner->wake();if(owner->thread.joinable())owner->thread.join();}
extern "C" size_t weavelo_windows_screen_capture_count(UINT64 handle,UINT32 source) {
    std::lock_guard<std::mutex> lock(roots_mutex);size_t count=0;
    for(const auto& entry:roots)if(std::get<0>(entry.first)==handle&&std::get<1>(entry.first)==source&&!entry.second.expired())++count;return count;
}

// An explicit test-only entry: synthetic pixels are uploaded once, then every
// crop uses the product's D3D11 video processor and one real hardware encoder.
extern "C" int weavelo_windows_screen_test_gpu(void* context,MakePlan plan,DetailedPacket packet,NativeError error) {
    try {
        Apartment apartment;MediaFoundation foundation;Root root([]()->unsigned long long{return 1;});
        const UINT32 width=640,height=360;std::vector<UINT32> pixels(width*height);
        const UINT32 colors[]={0xffff0000,0xff00ff00,0xff0000ff,0xffffffff};
        for(UINT32 y=0;y<height;++y)for(UINT32 x=0;x<width;++x)pixels[y*width+x]=colors[(y>=180?2:0)+(x>=320?1:0)];
        auto frame=std::make_shared<RawFrame>();frame->width=width;frame->height=height;
        D3D11_TEXTURE2D_DESC desc{};desc.Width=width;desc.Height=height;desc.MipLevels=1;desc.ArraySize=1;
        desc.Format=DXGI_FORMAT_B8G8R8A8_UNORM;desc.SampleDesc.Count=1;desc.Usage=D3D11_USAGE_DEFAULT;desc.BindFlags=D3D11_BIND_RENDER_TARGET;
        D3D11_SUBRESOURCE_DATA initial{pixels.data(),width*4,0};check(root.gpu.device->CreateTexture2D(&desc,&initial,&frame->texture),"Test native source texture");
        std::unique_ptr<Encoder> encoder;std::unique_ptr<Cropper> cropper;RECT full{0,0,LONG(width),LONG(height)};
        const auto deadline=Clock::now()+std::chrono::seconds(8);
        for(unsigned n=0;n<8;++n) {
            Plan next{};if(!plan(context,width,height,&next))throw Failure{E_FAIL,"Test viewport plan"};
            if(next.width!=320||next.height!=180||next.crop_width!=320||next.crop_height!=180||next.x+320>width||next.y+180>height)
                throw Failure{E_BOUNDS,"Test viewport dimensions"};
            if(!encoder){encoder=std::make_unique<Encoder>(root.gpu,320,180,60,4'000'000,Signal{MFVideoTransferMatrix_BT709,MFVideoPrimaries_BT709,MFVideoTransFunc_sRGB,MFNominalRange_16_235});
                cropper=std::make_unique<Cropper>(root,width,height,next);}
            while(!encoder->ready()||!cropper->available()){encoder->drain(nullptr,context,packet);if(Clock::now()>deadline)throw Failure{E_FAIL,"Test HEVC admission timeout"};std::this_thread::sleep_for(std::chrono::milliseconds(1));}
            frame->captured=n+1;encoder->submit(cropper->crop(frame,full,next,(n+1)*166667));encoder->drain(nullptr,context,packet);
        }
        encoder->finish_input();
        while(encoder->inflight()){encoder->drain(nullptr,context,packet);if(Clock::now()>deadline)throw Failure{E_FAIL,"Test HEVC output timeout"};std::this_thread::sleep_for(std::chrono::milliseconds(1));}
        return 1;
    }catch(Failure f){char message[180];std::snprintf(message,sizeof(message),"%s (0x%08lx)",f.stage,static_cast<unsigned long>(f.hr));error(context,message);}
    catch(...){error(context,"Native viewport GPU test failed");}return 0;
}
