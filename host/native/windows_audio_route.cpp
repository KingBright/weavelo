// Windows endpoint enumeration and temporary default-render routing.
// SetDefaultEndpoint uses the undocumented IPolicyConfig ABI; Windows does not
// expose a supported public API for changing the system default render endpoint.
#include <windows.h>
#include <mmdeviceapi.h>
#include <mmreg.h>
#include <functiondiscoverykeys_devpkey.h>
#include <cfgmgr32.h>
#include <propvarutil.h>
#include <string>
#include <vector>
#include <new>

namespace {
struct IPolicyConfig : IUnknown {
    virtual HRESULT STDMETHODCALLTYPE GetMixFormat(PCWSTR, WAVEFORMATEX**) = 0;
    virtual HRESULT STDMETHODCALLTYPE GetDeviceFormat(PCWSTR, INT, WAVEFORMATEX**) = 0;
    virtual HRESULT STDMETHODCALLTYPE ResetDeviceFormat(PCWSTR) = 0;
    virtual HRESULT STDMETHODCALLTYPE SetDeviceFormat(PCWSTR, WAVEFORMATEX*, WAVEFORMATEX*) = 0;
    virtual HRESULT STDMETHODCALLTYPE GetProcessingPeriod(PCWSTR, INT, PINT64, PINT64) = 0;
    virtual HRESULT STDMETHODCALLTYPE SetProcessingPeriod(PCWSTR, PINT64) = 0;
    virtual HRESULT STDMETHODCALLTYPE GetShareMode(PCWSTR, void*) = 0;
    virtual HRESULT STDMETHODCALLTYPE SetShareMode(PCWSTR, void*) = 0;
    virtual HRESULT STDMETHODCALLTYPE GetPropertyValue(PCWSTR, const PROPERTYKEY&, PROPVARIANT*) = 0;
    virtual HRESULT STDMETHODCALLTYPE SetPropertyValue(PCWSTR, const PROPERTYKEY&, const PROPVARIANT&) = 0;
    virtual HRESULT STDMETHODCALLTYPE SetDefaultEndpoint(PCWSTR, ERole) = 0;
    virtual HRESULT STDMETHODCALLTYPE SetEndpointVisibility(PCWSTR, INT) = 0;
};
const CLSID policy_class = {0x870af99c,0x171d,0x4f9e,{0xaf,0x0d,0xe6,0x3d,0xf4,0x0c,0x2b,0xc9}};
// DEVPROPKEY values from the Windows SDK; local constants avoid requiring a
// process-wide INITGUID definition or additional device-key object library.
const DEVPROPKEY hardware_key = {{0xa45c254e,0xdf1c,0x4efd,{0x80,0x20,0x67,0xd1,0x46,0xa8,0x50,0xe0}},3};
const DEVPROPKEY manufacturer_key = {{0xa45c254e,0xdf1c,0x4efd,{0x80,0x20,0x67,0xd1,0x46,0xa8,0x50,0xe0}},13};
const DEVPROPKEY enumerator_key = {{0xa45c254e,0xdf1c,0x4efd,{0x80,0x20,0x67,0xd1,0x46,0xa8,0x50,0xe0}},24};
// Endpoint store's topology device instance is prefixed by "{1}." on
// Windows. This mapping is optional and must not be used as proof of type.
const PROPERTYKEY topology_instance_key = {{0xb3f8fa53,0x0004,0x438e,{0x90,0x03,0x51,0xa4,0x6e,0x13,0x9b,0xfc}},2};
const IID policy_iid = {0xf8679f50,0x850a,0x41cf,{0x9c,0x72,0x43,0x0f,0x29,0x02,0x90,0xc8}};

template<class T> struct Com {
    T* p = nullptr;
    ~Com() { if (p) p->Release(); }
    T** out() { return &p; }
    T* operator->() const { return p; }
    void reset() { if (p) p->Release(); p = nullptr; }
    Com() = default;
    Com(const Com&) = delete; Com& operator=(const Com&) = delete;
};
struct Apartment {
    HRESULT result = CoInitializeEx(nullptr, COINIT_MULTITHREADED);
    ~Apartment() { if (SUCCEEDED(result)) CoUninitialize(); }
};
std::wstring property(IPropertyStore* store, const PROPERTYKEY& key) {
    PROPVARIANT v; PropVariantInit(&v);
    std::wstring result;
    if (SUCCEEDED(store->GetValue(key, &v)) && v.vt == VT_LPWSTR && v.pwszVal) result = v.pwszVal;
    PropVariantClear(&v);
    return result;
}
std::wstring dev_property(DEVINST node, const DEVPROPKEY& key) {
    DEVPROPTYPE type = 0; ULONG bytes = 0;
    if (CM_Get_DevNode_PropertyW(node, &key, &type, nullptr, &bytes, 0) != CR_BUFFER_SMALL || bytes > 65536 || !bytes) return {};
    std::vector<BYTE> buf(bytes + sizeof(wchar_t) * 2, 0);
    if (CM_Get_DevNode_PropertyW(node, &key, &type, buf.data(), &bytes, 0) != CR_SUCCESS) return {};
    if (type != DEVPROP_TYPE_STRING && type != DEVPROP_TYPE_STRING_LIST) return {};
    auto text = reinterpret_cast<const wchar_t*>(buf.data());
    // For a string list expose its first hardware ID, not a guessed class.
    return text;
}
HRESULT enumerator(IMMDeviceEnumerator** out) {
    return CoCreateInstance(__uuidof(MMDeviceEnumerator), nullptr, CLSCTX_ALL, __uuidof(IMMDeviceEnumerator), reinterpret_cast<void**>(out));
}
HRESULT default_id(IMMDeviceEnumerator* e, ERole role, std::wstring& id) {
    Com<IMMDevice> device;
    HRESULT hr = e->GetDefaultAudioEndpoint(eRender, role, device.out());
    if (FAILED(hr)) return hr;
    LPWSTR value = nullptr;
    hr = device->GetId(&value);
    if (SUCCEEDED(hr) && value) id = value;
    CoTaskMemFree(value);
    return hr;
}
struct Route {
    Apartment apartment;
    Com<IMMDeviceEnumerator> enumerator;
    Com<IPolicyConfig> policy;
    std::wstring target, before[3];
    bool owned[3] = {false, false, false};
};
HRESULT restore(Route* route, unsigned* failed) {
    HRESULT first = S_OK; *failed = 0;
    for (int i=0;i<3;++i) {
        if (!route->owned[i]) continue;
        std::wstring current;
        HRESULT hr = default_id(route->enumerator.p, static_cast<ERole>(i), current);
        if (SUCCEEDED(hr) && current == route->target) hr = route->policy->SetDefaultEndpoint(route->before[i].c_str(), static_cast<ERole>(i));
        if (FAILED(hr)) { *failed |= 1u << i; if (SUCCEEDED(first)) first = hr; }
        else route->owned[i] = false; // A different current default is a user change.
    }
    return first;
}
}

extern "C" {
// All strings are valid only during callback. Metadata may be empty when Windows
// cannot expose it. Hardware is the driver matching-device ID when available,
// otherwise the first hardware ID on the mapped device/ancestor.
typedef void (*weavelo_endpoint_callback)(void*, const wchar_t*, const wchar_t*, const wchar_t*, const wchar_t*, const wchar_t*, const wchar_t*);
long weavelo_win_audio_endpoints(weavelo_endpoint_callback callback, void* context) {
    if (!callback) return E_INVALIDARG;
    Apartment apartment; if (FAILED(apartment.result)) return apartment.result;
    Com<IMMDeviceEnumerator> e; HRESULT hr = enumerator(e.out()); if (FAILED(hr)) return hr;
    Com<IMMDeviceCollection> devices; hr = e->EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE, devices.out()); if (FAILED(hr)) return hr;
    UINT count=0; hr = devices->GetCount(&count); if (FAILED(hr)) return hr;
    for (UINT i=0;i<count;++i) {
        Com<IMMDevice> device; hr=devices->Item(i,device.out()); if (FAILED(hr)) return hr;
        LPWSTR raw=nullptr; hr=device->GetId(&raw); if (FAILED(hr)) return hr;
        std::wstring id = raw ? raw : L""; CoTaskMemFree(raw);
        Com<IPropertyStore> props; hr=device->OpenPropertyStore(STGM_READ, props.out()); if (FAILED(hr)) return hr;
        std::wstring name=property(props.p,PKEY_Device_FriendlyName);
        std::wstring instance=property(props.p,topology_instance_key);
        if (instance.compare(0, 4, L"{1}.")==0) instance.erase(0, 4);
        else instance.clear();
        std::wstring hardware=property(props.p,PKEY_Device_MatchingDeviceId);
        std::wstring provider=property(props.p,PKEY_Device_DriverProvider);
        std::wstring bus=property(props.p,PKEY_Device_EnumeratorName);
        DEVINST node;
        if (!instance.empty() && CM_Locate_DevNodeW(&node, const_cast<wchar_t*>(instance.c_str()), CM_LOCATE_DEVNODE_NORMAL)==CR_SUCCESS) {
            for (unsigned depth=0;depth<16;++depth) {
                auto candidate=dev_property(node,hardware_key);
                if (!candidate.empty()) {
                    if (hardware.empty()) hardware=candidate;
                    if (provider.empty()) provider=dev_property(node,manufacturer_key);
                    if (bus.empty()) bus=dev_property(node,enumerator_key);
                    break;
                }
                DEVINST parent; if (CM_Get_Parent(&parent,node,0)!=CR_SUCCESS) break; node=parent;
            }
        }
        callback(context,id.c_str(),name.c_str(),instance.c_str(),hardware.c_str(),provider.c_str(),bus.c_str());
    }
    return S_OK;
}
// Report all three render defaults before opening the worker so callers can
// bind local playback explicitly. A missing role is an error, not a guess.
typedef void (*weavelo_default_callback)(void*, unsigned, const wchar_t*);
long weavelo_win_audio_defaults(weavelo_default_callback callback, void* context) {
    if (!callback) return E_INVALIDARG;
    Apartment apartment; if (FAILED(apartment.result)) return apartment.result;
    Com<IMMDeviceEnumerator> e; HRESULT hr=enumerator(e.out()); if (FAILED(hr)) return hr;
    for (unsigned i=0;i<3;++i) {
        std::wstring id; hr=default_id(e.p,static_cast<ERole>(i),id);
        if (FAILED(hr)) return hr;
        callback(context,i,id.c_str());
    }
    return S_OK;
}
// Non-mutating probe for this undocumented interface only; it does not prove a
// later SetDefaultEndpoint will succeed on this OS/device.
long weavelo_win_audio_policy_probe() {
    Apartment apartment; if (FAILED(apartment.result)) return apartment.result;
    Com<IPolicyConfig> policy;
    return CoCreateInstance(policy_class,nullptr,CLSCTX_ALL,policy_iid,reinterpret_cast<void**>(policy.out()));
}
// Must start, restore, and destroy on the same thread. Missing original default
// or invalid/inactive target fails before any mutation. On partial failure an
// immediate owned-role rollback is attempted; its status is returned separately.
long weavelo_win_audio_route_start(const wchar_t* target, void** out, long* rollback_error, unsigned* rollback_roles) {
    if (!target || !*target || !out || !rollback_error || !rollback_roles) return E_INVALIDARG;
    *out=nullptr; *rollback_error=S_OK; *rollback_roles=0;
    Route* r=new(std::nothrow) Route(); if (!r) return E_OUTOFMEMORY;
    HRESULT hr=r->apartment.result;
    if (SUCCEEDED(hr)) hr=enumerator(r->enumerator.out());
    Com<IMMDevice> selected;
    if (SUCCEEDED(hr)) hr=r->enumerator->GetDevice(target,selected.out());
    DWORD state=0; if (SUCCEEDED(hr)) hr=selected->GetState(&state);
    if (SUCCEEDED(hr) && state!=DEVICE_STATE_ACTIVE) hr=HRESULT_FROM_WIN32(ERROR_DEVICE_NOT_CONNECTED);
    if (SUCCEEDED(hr)) hr=CoCreateInstance(policy_class,nullptr,CLSCTX_ALL,policy_iid,reinterpret_cast<void**>(r->policy.out()));
    for (int i=0;SUCCEEDED(hr) && i<3;++i) hr=default_id(r->enumerator.p,static_cast<ERole>(i),r->before[i]);
    r->target=target;
    for (int i=0;SUCCEEDED(hr) && i<3;++i) {
        if (r->before[i]==r->target) continue;
        // Refuse to overwrite a change made while acquiring the snapshot.
        std::wstring current; hr=default_id(r->enumerator.p,static_cast<ERole>(i),current);
        if (SUCCEEDED(hr) && current!=r->before[i]) hr=HRESULT_FROM_WIN32(ERROR_RETRY);
        if (SUCCEEDED(hr)) {
            hr=r->policy->SetDefaultEndpoint(target,static_cast<ERole>(i));
            if (SUCCEEDED(hr)) {
                r->owned[i]=true;
                std::wstring observed;
                hr=default_id(r->enumerator.p,static_cast<ERole>(i),observed);
                if (SUCCEEDED(hr) && observed!=r->target) hr=HRESULT_FROM_WIN32(ERROR_RETRY);
            }
        }
    }
    if (FAILED(hr)) {
        selected.reset();
        if (r->policy.p && r->enumerator.p) *rollback_error=restore(r,rollback_roles);
        delete r; return hr;
    }
    *out=r; return S_OK;
}
// Failed roles remain owned so the caller may retry before destroying the guard.
long weavelo_win_audio_route_restore(void* handle, unsigned* failed_roles) {
    if (!handle || !failed_roles) return E_INVALIDARG;
    return restore(static_cast<Route*>(handle),failed_roles);
}
void weavelo_win_audio_route_destroy(void* handle) { delete static_cast<Route*>(handle); }
}
