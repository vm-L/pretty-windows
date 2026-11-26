use std::cell::RefCell;
use std::mem::size_of;
use windows::core::{implement, Interface, Result, GUID, HSTRING, PCWSTR};
use windows::Foundation::{IPropertyValue, PropertyValue};
use windows::Graphics::Effects::{
    IGraphicsEffect, IGraphicsEffectSource, IGraphicsEffectSource_Impl, IGraphicsEffect_Impl,
};
use windows::System::{DispatcherQueue, DispatcherQueueController};
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::{
    CompositionEffectBrush, CompositionEffectFactoryLoadStatus, CompositionEffectSourceParameter,
    Compositor, SpriteVisual,
};
use windows::Win32::Foundation::{E_INVALIDARG, HWND};
use windows::Win32::Graphics::Direct2D::Common::D2D1_BORDER_MODE_HARD;
use windows::Win32::Graphics::Direct2D::{
    CLSID_D2D1GaussianBlur, CLSID_D2D1Saturation, D2D1_GAUSSIANBLUR_OPTIMIZATION_BALANCED,
};
use windows::Win32::System::WinRT::Composition::ICompositorDesktopInterop;
use windows::Win32::System::WinRT::Graphics::Direct2D::{
    IGraphicsEffectD2D1Interop, IGraphicsEffectD2D1Interop_Impl, GRAPHICS_EFFECT_PROPERTY_MAPPING,
};
use windows::Win32::System::WinRT::{
    CreateDispatcherQueueController, DispatcherQueueOptions, DQTAT_COM_NONE,
    DQTYPE_THREAD_CURRENT,
};

// ---------- WinRT Composition Direct2D Effect Interop ----------

#[implement(IGraphicsEffect, IGraphicsEffectSource, IGraphicsEffectD2D1Interop)]
struct GaussianBlurEffect {
    name: HSTRING,
    sigma: f32,
    source: IGraphicsEffectSource,
}

impl IGraphicsEffectSource_Impl for GaussianBlurEffect_Impl {}

impl IGraphicsEffect_Impl for GaussianBlurEffect_Impl {
    fn Name(&self) -> Result<HSTRING> {
        Ok(self.name.clone())
    }

    fn SetName(&self, _name: &HSTRING) -> Result<()> {
        Ok(())
    }
}

impl IGraphicsEffectD2D1Interop_Impl for GaussianBlurEffect_Impl {
    fn GetEffectId(&self) -> Result<GUID> {
        Ok(CLSID_D2D1GaussianBlur)
    }

    fn GetNamedPropertyMapping(
        &self,
        _name: &PCWSTR,
        _index: *mut u32,
        _mapping: *mut GRAPHICS_EFFECT_PROPERTY_MAPPING,
    ) -> Result<()> {
        Err(E_INVALIDARG.into())
    }

    fn GetPropertyCount(&self) -> Result<u32> {
        Ok(3)
    }

    fn GetProperty(&self, index: u32) -> Result<IPropertyValue> {
        match index {
            0 => PropertyValue::CreateSingle(self.sigma)?.cast(),
            1 => PropertyValue::CreateUInt32(D2D1_GAUSSIANBLUR_OPTIMIZATION_BALANCED.0 as u32)?
                .cast(),
            2 => PropertyValue::CreateUInt32(D2D1_BORDER_MODE_HARD.0 as u32)?.cast(),
            _ => Err(E_INVALIDARG.into()),
        }
    }

    fn GetSource(&self, index: u32) -> Result<IGraphicsEffectSource> {
        if index == 0 {
            Ok(self.source.clone())
        } else {
            Err(E_INVALIDARG.into())
        }
    }

    fn GetSourceCount(&self) -> Result<u32> {
        Ok(1)
    }
}

#[implement(IGraphicsEffect, IGraphicsEffectSource, IGraphicsEffectD2D1Interop)]
struct SaturationEffect {
    name: HSTRING,
    saturation: f32,
    source: IGraphicsEffectSource,
}

impl IGraphicsEffectSource_Impl for SaturationEffect_Impl {}

impl IGraphicsEffect_Impl for SaturationEffect_Impl {
    fn Name(&self) -> Result<HSTRING> {
        Ok(self.name.clone())
    }

    fn SetName(&self, _name: &HSTRING) -> Result<()> {
        Ok(())
    }
}

impl IGraphicsEffectD2D1Interop_Impl for SaturationEffect_Impl {
    fn GetEffectId(&self) -> Result<GUID> {
        Ok(CLSID_D2D1Saturation)
    }

    fn GetNamedPropertyMapping(
        &self,
        _name: &PCWSTR,
        _index: *mut u32,
        _mapping: *mut GRAPHICS_EFFECT_PROPERTY_MAPPING,
    ) -> Result<()> {
        Err(E_INVALIDARG.into())
    }

    fn GetPropertyCount(&self) -> Result<u32> {
        Ok(1)
    }

    fn GetProperty(&self, index: u32) -> Result<IPropertyValue> {
        if index == 0 {
            PropertyValue::CreateSingle(self.saturation)?.cast()
        } else {
            Err(E_INVALIDARG.into())
        }
    }

    fn GetSource(&self, index: u32) -> Result<IGraphicsEffectSource> {
        if index == 0 {
            Ok(self.source.clone())
        } else {
            Err(E_INVALIDARG.into())
        }
    }

    fn GetSourceCount(&self) -> Result<u32> {
        Ok(1)
    }
}

thread_local! {
    /// Per-UI-thread `DispatcherQueueController` shared across all `CompositionEngine` instances
    /// (taskbar + deskbox) so recreating one engine never tears down the thread's `DispatcherQueue`
    /// while another module's `Compositor` / `DesktopWindowTarget` is still live.
    static THREAD_DQ_CONTROLLER: RefCell<Option<DispatcherQueueController>> =
        const { RefCell::new(None) };
}

fn ensure_thread_dispatcher_queue() -> bool {
    if DispatcherQueue::GetForCurrentThread().is_ok() {
        return true;
    }
    let opts = DispatcherQueueOptions {
        dwSize: size_of::<DispatcherQueueOptions>() as u32,
        threadType: DQTYPE_THREAD_CURRENT,
        apartmentType: DQTAT_COM_NONE,
    };
    let Ok(controller) = (unsafe { CreateDispatcherQueueController(opts) }) else {
        return false;
    };
    let _ = THREAD_DQ_CONTROLLER.try_with(|slot| {
        if let Ok(mut guard) = slot.try_borrow_mut() {
            *guard = Some(controller);
        }
    });
    true
}

/// Shared WinRT `Windows.UI.Composition` real-time desktop backdrop blur engine.
pub struct CompositionEngine {
    desktop_interop: ICompositorDesktopInterop,
    pub compositor: Compositor,
}

impl CompositionEngine {
    /// Releases the thread-local WinRT `DispatcherQueueController` before `OleUninitialize` /
    /// `CoUninitialize` so COM/WinRT objects are never released during late CRT/TLS teardown.
    pub fn shutdown_thread_dispatcher_queue() {
        let _ = THREAD_DQ_CONTROLLER.try_with(|slot| {
            if let Ok(mut guard) = slot.try_borrow_mut() {
                drop(guard.take());
            }
        });
    }

    /// Initializes a per-thread `Compositor` and `DispatcherQueue` (when needed) for live HostBackdrop effects.
    pub fn try_new() -> Option<Self> {
        if !ensure_thread_dispatcher_queue() {
            return None;
        }
        let compositor = Compositor::new().ok()?;
        let desktop_interop: ICompositorDesktopInterop = compositor.cast().ok()?;
        Some(Self {
            desktop_interop,
            compositor,
        })
    }

    /// Creates a composition target and root `SpriteVisual` bound to `hwnd`.
    pub fn create_target_and_visual(&self, hwnd: HWND) -> Option<(DesktopWindowTarget, SpriteVisual)> {
        let target = unsafe {
            self.desktop_interop
                .CreateDesktopWindowTarget(hwnd, false)
                .ok()?
        };
        let visual = self.compositor.CreateSpriteVisual().ok()?;
        target.SetRoot(&visual).ok()?;
        Some((target, visual))
    }

    /// Builds a `GaussianBlur` + `Saturation` `CompositionEffectBrush` driven by `CreateHostBackdropBrush`.
    pub fn build_backdrop_brush(
        &self,
        sigma_px: f32,
        saturation: f32,
    ) -> Option<CompositionEffectBrush> {
        let param_name = HSTRING::from("backdrop");
        let source_param = CompositionEffectSourceParameter::Create(&param_name).ok()?;
        let source_iface: IGraphicsEffectSource = source_param.cast().ok()?;
        let sigma = sigma_px.max(0.5);

        let blur_effect: IGraphicsEffect = GaussianBlurEffect {
            name: HSTRING::from("Blur"),
            sigma,
            source: source_iface.clone(),
        }
        .into();

        let factory = if (saturation - 1.0).abs() >= 0.01 {
            let blur_source: IGraphicsEffectSource = blur_effect.cast().ok()?;
            let sat_effect: IGraphicsEffect = SaturationEffect {
                name: HSTRING::from("Saturation"),
                saturation: saturation.max(0.0),
                source: blur_source,
            }
            .into();
            self.compile_effect_factory(&sat_effect)
                .or_else(|| self.compile_effect_factory(&blur_effect))?
        } else {
            self.compile_effect_factory(&blur_effect)?
        };

        let brush = factory.CreateBrush().ok()?;
        let host_backdrop = self.compositor.CreateHostBackdropBrush().ok()?;
        brush.SetSourceParameter(&param_name, &host_backdrop).ok()?;
        Some(brush)
    }

    fn compile_effect_factory(
        &self,
        effect: &IGraphicsEffect,
    ) -> Option<windows::UI::Composition::CompositionEffectFactory> {
        let factory = self.compositor.CreateEffectFactory(effect).ok()?;
        let status = factory.LoadStatus().ok()?;
        if matches!(
            status,
            CompositionEffectFactoryLoadStatus::EffectTooComplex
                | CompositionEffectFactoryLoadStatus::Other
        ) {
            return None;
        }
        Some(factory)
    }
}
