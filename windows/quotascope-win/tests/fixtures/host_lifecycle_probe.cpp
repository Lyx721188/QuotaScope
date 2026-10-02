// Independent C++/WinRT diagnostic: no Rust, Reactor, or QuotaScope state.
// See Docs/windows-settings-lifecycle.md for SDK/build details and observations.
// PROBE_CONTROLS=text|nav|combo; PROBE_ROUNDS defaults to 2.
// PROBE_RESET_MUXC invokes a PRIVATE export solely as a causal experiment.
// Do not introduce this cleanup hook into the production application.
#include <windows.h>
#undef GetCurrentTime
#include <winrt/Windows.Foundation.h>
#include <winrt/Windows.Foundation.Collections.h>
#include <winrt/Windows.UI.Xaml.Interop.h>
#include <winrt/Microsoft.UI.Xaml.h>
#include <winrt/Microsoft.UI.Xaml.Controls.h>
#include <winrt/Microsoft.UI.Xaml.Markup.h>
#include <winrt/Microsoft.UI.Xaml.XamlTypeInfo.h>
#include <thread>
#include <chrono>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <cwchar>

namespace xaml=winrt::Microsoft::UI::Xaml;
namespace controls=xaml::Controls;
namespace markup=xaml::Markup;

struct DiagnosticApp : xaml::ApplicationT<DiagnosticApp,markup::IXamlMetadataProvider> {
    xaml::XamlTypeInfo::XamlControlsXamlMetaDataProvider provider{nullptr};
    xaml::Window window{nullptr};
    auto Provider() {
        if (!provider) provider=xaml::XamlTypeInfo::XamlControlsXamlMetaDataProvider{};
        return provider;
    }
    markup::IXamlType GetXamlType(winrt::Windows::UI::Xaml::Interop::TypeName const& type) {
        return Provider().GetXamlType(type);
    }
    markup::IXamlType GetXamlType(winrt::hstring const& name) {
        return Provider().GetXamlType(name);
    }
    winrt::com_array<markup::XmlnsDefinition> GetXmlnsDefinitions() {
        return Provider().GetXmlnsDefinitions();
    }
    void OnLaunched(xaml::LaunchActivatedEventArgs const&) {
        Resources().MergedDictionaries().Append(controls::XamlControlsResources{});
        window=xaml::Window{};
        window.Title(L"C++ WinUI restart diagnostic");
        auto name=std::getenv("PROBE_CONTROLS");
        if (name && std::strcmp(name,"nav")==0) window.Content(controls::NavigationView{});
        else if (name && std::strcmp(name,"combo")==0) {
            controls::ComboBox combo;
            combo.Items().Append(winrt::box_value(L"A"));
            combo.Items().Append(winrt::box_value(L"B"));
            window.Content(combo);
        } else {
            controls::TextBlock text;
            text.Text(L"Independent C++/WinRT diagnostic; no Rust or Reactor");
            window.Content(text);
        }
        window.Activate();
        auto thread=GetCurrentThreadId();
        std::thread([thread] {
            std::this_thread::sleep_for(std::chrono::seconds(2));
            EnumThreadWindows(thread,[](HWND hwnd,LPARAM)->BOOL {
                wchar_t title[128]{};
                GetWindowTextW(hwnd,title,128);
                if (std::wcscmp(title,L"C++ WinUI restart diagnostic")==0) PostMessageW(hwnd,WM_CLOSE,0,0);
                return TRUE;
            },0);
        }).detach();
    }
};

int main() {
    try {
        winrt::init_apartment(winrt::apartment_type::single_threaded);
        auto count=std::getenv("PROBE_ROUNDS");
        auto rounds=count?std::atoi(count):2;
        for (int i=1;i<=rounds;i++) {
            xaml::Application application{nullptr};
            std::printf("CPP Application::Start attempt %d\n",i);std::fflush(stdout);
            xaml::Application::Start([&](auto&&){application=winrt::make<DiagnosticApp>();});
            application=nullptr;
            std::printf("CPP Application::Start attempt %d returned Ok\n",i);std::fflush(stdout);
            if (std::getenv("PROBE_RESET_MUXC") && i<rounds) {
                auto module=GetModuleHandleW(L"Microsoft.UI.Xaml.Controls.dll");
                auto cleanup=reinterpret_cast<void(__stdcall*)()>(GetProcAddress(module,"DeinitializeMUXC"));
                if (!cleanup) return 2;
                cleanup();
                std::printf("DIAGNOSTIC: DeinitializeMUXC after completed host %d\n",i);std::fflush(stdout);
            }
        }
        std::puts("PASS: independent C++ WinUI host restart");
        return 0;
    } catch (winrt::hresult_error const& e) {
        std::printf("C++ HRESULT %08x\n",static_cast<unsigned int>(e.code().value));
        return 3;
    }
}
