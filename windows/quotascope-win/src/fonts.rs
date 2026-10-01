//! Bundled HarmonyOS Sans; no installation or changes to system fonts.
use windows::core::{IInspectable, IInspectable_Vtbl, Interface, Result, HRESULT, HSTRING};
use windows::Foundation::{PropertyValue, Uri};
use windows::Win32::Graphics::DirectWrite::{
    IDWriteFactory, IDWriteFactory5, IDWriteFontCollection,
};
use windows::Win32::System::WinRT::{RoActivateInstance, RoGetActivationFactory};
use windows_collections::IMap;

pub const FAMILY: &str = "HarmonyOS Sans SC";

pub fn collection(factory: &IDWriteFactory) -> Result<IDWriteFontCollection> {
    #[cfg(not(test))]
    let base = std::env::current_exe()?.parent().unwrap().join("Fonts");
    #[cfg(test)]
    let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/fonts");
    collection_at(factory, &base)
}

fn collection_at(
    factory: &IDWriteFactory,
    base: &std::path::Path,
) -> Result<IDWriteFontCollection> {
    unsafe {
        let modern: IDWriteFactory5 = factory.cast()?;
        let builder = modern.CreateFontSetBuilder()?;
        for weight in ["Regular", "Medium", "Bold"] {
            let path = base.join(format!("HarmonyOS_Sans_SC_{weight}.ttf"));
            let wide = HSTRING::from(path.to_string_lossy().as_ref());
            let file =
                factory.CreateFontFileReference(windows::core::PCWSTR(wide.as_ptr()), None)?;
            builder.AddFontFile(&file)?;
        }
        let fonts = builder.CreateFontSet()?;
        let collection: IDWriteFontCollection =
            modern.CreateFontCollectionFromFontSet(&fonts)?.cast()?;
        let mut index = 0;
        let mut exists = windows::core::BOOL::default();
        collection.FindFamilyName(&HSTRING::from(FAMILY), &mut index, &mut exists)?;
        if !exists.as_bool() {
            return Err(windows::core::Error::from_hresult(
                windows::Win32::Foundation::E_FAIL,
            ));
        }
        Ok(collection)
    }
}

// Windows Reactor does not expose FontFamily properties yet. These three
// stable WinUI ABI interfaces only override application font resources;
// their IIDs and vtable order match Reactor's generated WinUI bindings.
windows::core::imp::define_interface!(
    ApplicationStatics,
    ApplicationStaticsVtbl,
    0x4e0d09f5_4358_512c_a987_503b52848e95
);
#[repr(C)]
pub struct ApplicationStaticsVtbl {
    base: IInspectable_Vtbl,
    current:
        unsafe extern "system" fn(*mut core::ffi::c_void, *mut *mut core::ffi::c_void) -> HRESULT,
}
windows::core::imp::define_interface!(
    Application,
    ApplicationVtbl,
    0x06a8f4e7_1146_55af_820d_ebd55643b021
);
#[repr(C)]
pub struct ApplicationVtbl {
    base: IInspectable_Vtbl,
    resources:
        unsafe extern "system" fn(*mut core::ffi::c_void, *mut *mut core::ffi::c_void) -> HRESULT,
}
windows::core::imp::define_interface!(
    ResourceDictionary,
    ResourceDictionaryVtbl,
    0x1b690975_a710_5783_a6e1_15836f6186c2
);
#[repr(C)]
pub struct ResourceDictionaryVtbl {
    base: IInspectable_Vtbl,
    source: usize,
    set_source:
        unsafe extern "system" fn(*mut core::ffi::c_void, *mut core::ffi::c_void) -> HRESULT,
}

pub fn install_winui_font() -> Result<()> {
    unsafe {
        let statics: ApplicationStatics =
            RoGetActivationFactory(&HSTRING::from("Microsoft.UI.Xaml.Application"))?;
        let mut current = std::ptr::null_mut();
        (statics.vtable().current)(statics.as_raw(), &mut current).ok()?;
        if current.is_null() {
            return Err(windows::core::Error::from_hresult(
                windows::Win32::Foundation::E_UNEXPECTED,
            ));
        }
        let app: Application = IInspectable::from_raw(current).cast()?;
        let mut resources = std::ptr::null_mut();
        (app.vtable().resources)(app.as_raw(), &mut resources).ok()?;
        if resources.is_null() {
            return Err(windows::core::Error::from_hresult(
                windows::Win32::Foundation::E_UNEXPECTED,
            ));
        }
        let resources: IMap<IInspectable, IInspectable> =
            IInspectable::from_raw(resources).cast()?;
        let fonts: ResourceDictionary =
            RoActivateInstance(&HSTRING::from("Microsoft.UI.Xaml.ResourceDictionary"))?.cast()?;
        let source = Uri::CreateUri(&HSTRING::from("ms-appx:///Fonts/Fonts.xaml"))?;
        (fonts.vtable().set_source)(fonts.as_raw(), source.as_raw()).ok()?;
        let fonts: IMap<IInspectable, IInspectable> = fonts.cast()?;
        for name in [
            "ContentControlThemeFontFamily",
            "TextControlThemeFontFamily",
        ] {
            let key: IInspectable = PropertyValue::CreateString(&HSTRING::from(name))?.cast()?;
            resources.Insert(&key, &fonts.Lookup(&key)?)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::Graphics::DirectWrite::{DWriteCreateFactory, DWRITE_FACTORY_TYPE_SHARED};

    #[test]
    fn bundled_family_has_three_weights_and_chinese_glyphs() {
        unsafe {
            let factory: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED).unwrap();
            let fonts = collection_at(
                &factory,
                &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/fonts"),
            )
            .unwrap();
            assert_eq!(fonts.GetFontFamilyCount(), 1);
            let family = fonts.GetFontFamily(0).unwrap();
            let mut weights = Vec::new();
            for i in 0..family.GetFontCount() {
                let font = family.GetFont(i).unwrap();
                if font.GetSimulations()
                    == windows::Win32::Graphics::DirectWrite::DWRITE_FONT_SIMULATIONS_NONE
                {
                    weights.push(font.GetWeight().0);
                }
                let exists = font.HasCharacter('量' as u32).unwrap();
                assert!(exists.as_bool());
            }
            weights.sort();
            assert_eq!(weights, [400, 500, 700]);
        }
    }
}
