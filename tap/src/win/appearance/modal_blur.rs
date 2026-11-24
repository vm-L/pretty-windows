use super::super::raw_com::*;
use std::ffi::c_void;
use std::mem::size_of;
use std::ptr::null_mut;
use windows::core::{implement, Interface, Result, GUID, HSTRING, PCWSTR};
use windows::Foundation::Numerics::{Vector2, Vector3};
use windows::Foundation::{IPropertyValue, PropertyValue, TimeSpan};
use windows::Graphics::Effects::{
    IGraphicsEffect, IGraphicsEffectSource, IGraphicsEffectSource_Impl, IGraphicsEffect_Impl,
};
use windows::UI::Color;
use windows::UI::Composition::{
    AnimationIterationBehavior, CompositionAnimation, CompositionBrush, CompositionClip,
    CompositionEasingFunction, CompositionEffectBrush, CompositionEffectFactoryLoadStatus,
    CompositionEffectSourceParameter, CompositionGeometry, CompositionGradientExtendMode,
    CompositionMappingMode, CompositionObject, CompositionShape, Compositor, Visual,
};
use windows::Win32::Foundation::E_INVALIDARG;
use windows::Win32::Graphics::Direct2D::Common::D2D1_BORDER_MODE_HARD;
use windows::Win32::Graphics::Direct2D::{
    CLSID_D2D1GaussianBlur, CLSID_D2D1Saturation, D2D1_GAUSSIANBLUR_OPTIMIZATION_BALANCED,
};
use windows::Win32::System::WinRT::Graphics::Direct2D::{
    IGraphicsEffectD2D1Interop, IGraphicsEffectD2D1Interop_Impl, GRAPHICS_EFFECT_PROPERTY_MAPPING,
};

const SHADOW_PAD_DIP: f32 = 18.0;
const TIME_SPAN_TICKS_PER_SEC: i64 = 10_000_000;

#[derive(Clone, Copy)]
struct StartModalVisualRecipe {
    blur_sigma_dip: f32,
    saturation: f32,
    tint_color: Color,
    frost_veil: [f32; 3],
    enable_silk: bool,
    sheen_scale: f32,
    rim_stops: [f32; 3],
    rim_dark_sep: f32,
    rim_stroke_dip: f32,
    shadow_scale: f32,
}

const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color {
    Color { A: a, R: r, G: g, B: b }
}

fn alpha_u8(a: f32) -> u8 {
    (a.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn white_alpha(a: f32) -> Color {
    rgba(255, 255, 255, alpha_u8(a))
}

fn black_alpha(a: f32) -> Color {
    rgba(0, 0, 0, alpha_u8(a))
}

fn rgb_alpha(rgb: u32, a: f32) -> Color {
    rgba(
        ((rgb >> 16) & 0xFF) as u8,
        ((rgb >> 8) & 0xFF) as u8,
        (rgb & 0xFF) as u8,
        alpha_u8(a),
    )
}

fn lifted_pearl_rgb(c0: u32, c1: u32, light_glass: bool) -> u32 {
    let lift = if light_glass { 0.52_f32 } else { 0.42_f32 };
    let ch = |shift: u32| -> u32 {
        let v0 = ((c0 >> shift) & 0xFF) as f32 / 255.0;
        let v1 = ((c1 >> shift) & 0xFF) as f32 / 255.0;
        let mid = (v0 + v1) * 0.5;
        let lifted = mid + (1.0 - mid) * lift;
        ((lifted.clamp(0.0, 1.0) * 255.0).round() as u32) & 0xFF
    };
    (ch(16) << 16) | (ch(8) << 8) | ch(0)
}

const fn start_modal_visual_recipe(style_id: u8, light_glass: bool) -> StartModalVisualRecipe {
    match (style_id, light_glass) {
        // 1 = Transparent
        (1, false) => StartModalVisualRecipe {
            blur_sigma_dip: 4.5,
            saturation: 1.04,
            tint_color: rgba(36, 36, 38, 0x26),
            frost_veil: [0.03, 0.01, 0.0],
            enable_silk: false,
            sheen_scale: 0.30,
            rim_stops: [0.22, 0.12, 0.09],
            rim_dark_sep: 0.09,
            rim_stroke_dip: 1.05,
            shadow_scale: 0.65,
        },
        (1, true) => StartModalVisualRecipe {
            blur_sigma_dip: 4.5,
            saturation: 1.04,
            tint_color: rgba(250, 250, 251, 0x52),
            frost_veil: [0.07, 0.02, 0.0],
            enable_silk: false,
            sheen_scale: 0.30,
            rim_stops: [0.58, 0.28, 0.14],
            rim_dark_sep: 0.10,
            rim_stroke_dip: 1.0,
            shadow_scale: 0.65,
        },
        // 2 = Blur (Frosted Glass)
        (2, false) => StartModalVisualRecipe {
            blur_sigma_dip: 18.0,
            saturation: 1.08,
            tint_color: rgba(46, 46, 48, 0x46),
            frost_veil: [0.13, 0.05, 0.015],
            enable_silk: false,
            sheen_scale: 0.55,
            rim_stops: [0.26, 0.14, 0.10],
            rim_dark_sep: 0.09,
            rim_stroke_dip: 1.05,
            shadow_scale: 1.0,
        },
        (2, true) => StartModalVisualRecipe {
            blur_sigma_dip: 18.0,
            saturation: 1.08,
            tint_color: rgba(250, 250, 251, 0x7A),
            frost_veil: [0.22, 0.08, 0.03],
            enable_silk: false,
            sheen_scale: 0.55,
            rim_stops: [0.70, 0.34, 0.18],
            rim_dark_sep: 0.10,
            rim_stroke_dip: 1.0,
            shadow_scale: 1.0,
        },
        // 3 = Acrylic
        (3, false) => StartModalVisualRecipe {
            blur_sigma_dip: 26.0,
            saturation: 1.16,
            tint_color: rgba(38, 38, 40, 0x58),
            frost_veil: [0.06, 0.02, 0.0],
            enable_silk: false,
            sheen_scale: 0.45,
            rim_stops: [0.30, 0.16, 0.12],
            rim_dark_sep: 0.09,
            rim_stroke_dip: 1.05,
            shadow_scale: 1.0,
        },
        (3, true) => StartModalVisualRecipe {
            blur_sigma_dip: 26.0,
            saturation: 1.16,
            tint_color: rgba(248, 248, 250, 0x8E),
            frost_veil: [0.14, 0.05, 0.0],
            enable_silk: false,
            sheen_scale: 0.45,
            rim_stops: [0.76, 0.40, 0.22],
            rim_dark_sep: 0.10,
            rim_stroke_dip: 1.0,
            shadow_scale: 1.0,
        },
        // 4 / fallback = Liquid Glass
        (_, false) => StartModalVisualRecipe {
            blur_sigma_dip: 22.0,
            saturation: 1.34,
            tint_color: rgba(42, 42, 44, 0x36),
            frost_veil: [0.06, 0.025, 0.0],
            enable_silk: true,
            sheen_scale: 0.95,
            rim_stops: [0.26, 0.14, 0.10],
            rim_dark_sep: 0.09,
            rim_stroke_dip: 1.05,
            shadow_scale: 1.0,
        },
        (_, true) => StartModalVisualRecipe {
            blur_sigma_dip: 22.0,
            saturation: 1.22,
            tint_color: rgba(250, 250, 252, 0x6E),
            frost_veil: [0.15, 0.05, 0.0],
            enable_silk: true,
            sheen_scale: 0.95,
            rim_stops: [0.64, 0.30, 0.16],
            rim_dark_sep: 0.10,
            rim_stroke_dip: 1.0,
            shadow_scale: 1.0,
        },
    }
}

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

fn build_inapp_backdrop_blur_brush(
    compositor: &Compositor,
    sigma: f32,
    saturation: f32,
) -> Option<CompositionEffectBrush> {
    let param_name = HSTRING::from("backdrop");
    let source_param = CompositionEffectSourceParameter::Create(&param_name).ok()?;
    let source_iface: IGraphicsEffectSource = source_param.cast().ok()?;
    let blur_effect: IGraphicsEffect = GaussianBlurEffect {
        name: HSTRING::from("Blur"),
        sigma: sigma.max(0.5),
        source: source_iface,
    }
    .into();

    let compile_effect = |eff: &IGraphicsEffect| -> Option<CompositionEffectBrush> {
        let factory = compositor.CreateEffectFactory(eff).ok()?;
        let status = factory.LoadStatus().ok()?;
        if matches!(
            status,
            CompositionEffectFactoryLoadStatus::EffectTooComplex
                | CompositionEffectFactoryLoadStatus::Other
        ) {
            return None;
        }
        factory.CreateBrush().ok()
    };

    let brush = if (saturation - 1.0).abs() >= 0.01 {
        let blur_source: IGraphicsEffectSource = blur_effect.cast().ok()?;
        let sat_effect: IGraphicsEffect = SaturationEffect {
            name: HSTRING::from("Saturation"),
            saturation: saturation.max(0.0),
            source: blur_source,
        }
        .into();
        compile_effect(&sat_effect).or_else(|| compile_effect(&blur_effect))?
    } else {
        compile_effect(&blur_effect)?
    };

    let backdrop = compositor.CreateBackdropBrush().ok()?;
    let backdrop_brush: CompositionBrush = backdrop.cast().ok()?;
    brush.SetSourceParameter(&param_name, &backdrop_brush).ok()?;
    Some(brush)
}

fn make_size_expr_animation(
    compositor: &Compositor,
    host_obj: &CompositionObject,
    delta: Vector2,
) -> Option<CompositionAnimation> {
    let expr = if delta.X == 0.0 && delta.Y == 0.0 {
        compositor
            .CreateExpressionAnimationWithExpression(&HSTRING::from("host.Size"))
            .ok()?
    } else {
        let anim = compositor
            .CreateExpressionAnimationWithExpression(&HSTRING::from("host.Size + pad"))
            .ok()?;
        anim.SetVector2Parameter(&HSTRING::from("pad"), delta)
            .ok()?;
        anim
    };
    expr.SetReferenceParameter(&HSTRING::from("host"), host_obj)
        .ok()?;
    expr.cast().ok()
}

fn build_linear_gradient_brush(
    compositor: &Compositor,
    start: Vector2,
    end: Vector2,
    stops: &[(f32, Color)],
) -> Option<CompositionBrush> {
    let brush = compositor.CreateLinearGradientBrush().ok()?;
    brush
        .SetMappingMode(CompositionMappingMode::Relative)
        .ok()?;
    brush
        .SetExtendMode(CompositionGradientExtendMode::Clamp)
        .ok()?;
    brush.SetStartPoint(start).ok()?;
    brush.SetEndPoint(end).ok()?;
    let col_stops = brush.ColorStops().ok()?;
    for &(offset, color) in stops {
        let stop = compositor
            .CreateColorGradientStopWithOffsetAndColor(offset, color)
            .ok()?;
        col_stops.Append(&stop).ok()?;
    }
    brush.cast().ok()
}

fn make_full_sprite(compositor: &Compositor, brush: &CompositionBrush) -> Option<Visual> {
    let sprite = compositor.CreateSpriteVisual().ok()?;
    sprite
        .SetRelativeSizeAdjustment(Vector2 { X: 1.0, Y: 1.0 })
        .ok()?;
    sprite.SetBrush(brush).ok()?;
    sprite.cast().ok()
}

#[allow(clippy::too_many_arguments)]
fn build_silk_ribbon_sprite(
    compositor: &Compositor,
    easing: &CompositionEasingFunction,
    color_rgb: u32,
    peak_alpha: f32,
    center_start: Vector2,
    center_end: Vector2,
    radius: Vector2,
    duration_secs: i64,
) -> Option<Visual> {
    let brush = compositor.CreateRadialGradientBrush().ok()?;
    brush
        .SetMappingMode(CompositionMappingMode::Relative)
        .ok()?;
    brush
        .SetExtendMode(CompositionGradientExtendMode::Clamp)
        .ok()?;
    brush.SetEllipseCenter(center_start).ok()?;
    brush.SetEllipseRadius(radius).ok()?;

    let col_stops = brush.ColorStops().ok()?;
    let stops = [
        (0.0_f32, rgb_alpha(color_rgb, peak_alpha)),
        (0.38, rgb_alpha(color_rgb, peak_alpha * 0.72)),
        (0.72, rgb_alpha(color_rgb, peak_alpha * 0.24)),
        (1.0, rgb_alpha(color_rgb, 0.0)),
    ];
    for &(offset, color) in &stops {
        let stop = compositor
            .CreateColorGradientStopWithOffsetAndColor(offset, color)
            .ok()?;
        col_stops.Append(&stop).ok()?;
    }

    if let Ok(kfa) = compositor.CreateVector2KeyFrameAnimation() {
        let _ = kfa.SetDuration(TimeSpan {
            Duration: duration_secs.saturating_mul(TIME_SPAN_TICKS_PER_SEC),
        });
        let _ = kfa.SetIterationBehavior(AnimationIterationBehavior::Forever);
        let _ = kfa.InsertKeyFrame(0.0, center_start);
        let _ = kfa.InsertKeyFrameWithEasingFunction(0.5, center_end, easing);
        let _ = kfa.InsertKeyFrameWithEasingFunction(1.0, center_start, easing);
        if let Ok(anim_base) = kfa.cast::<CompositionAnimation>() {
            let _ = brush.StartAnimation(&HSTRING::from("EllipseCenter"), &anim_base);
        }
    }

    let base_brush: CompositionBrush = brush.cast().ok()?;
    make_full_sprite(compositor, &base_brush)
}

#[allow(clippy::too_many_arguments)]
fn build_clipped_glass_container(
    compositor: &Compositor,
    host_obj: &CompositionObject,
    corner_radius: f32,
    recipe: &StartModalVisualRecipe,
    halo0_rgb: u32,
    halo1_rgb: u32,
    light_glass: bool,
) -> Option<Visual> {
    let clipped = compositor.CreateContainerVisual().ok()?;
    clipped
        .SetRelativeSizeAdjustment(Vector2 { X: 1.0, Y: 1.0 })
        .ok()?;

    let rr_geo = compositor.CreateRoundedRectangleGeometry().ok()?;
    let r = corner_radius.max(0.0);
    rr_geo.SetCornerRadius(Vector2 { X: r, Y: r }).ok()?;
    let size_anim = make_size_expr_animation(compositor, host_obj, Vector2::default())?;
    rr_geo
        .StartAnimation(&HSTRING::from("Size"), &size_anim)
        .ok()?;
    let geo: CompositionGeometry = rr_geo.cast().ok()?;
    let geo_clip = compositor.CreateGeometricClipWithGeometry(&geo).ok()?;
    let clip_base: CompositionClip = geo_clip.cast().ok()?;
    clipped.SetClip(&clip_base).ok()?;

    let children = clipped.Children().ok()?;

    // Layer 1: In-app GPU backdrop Gaussian blur + optical saturation
    let blur_effect =
        build_inapp_backdrop_blur_brush(compositor, recipe.blur_sigma_dip, recipe.saturation)?;
    let blur_brush: CompositionBrush = blur_effect.cast().ok()?;
    let blur_vis = make_full_sprite(compositor, &blur_brush)?;
    children.InsertAtTop(&blur_vis).ok()?;

    // Layer 2: Calibrated translucent neutral tint (prevents muddy double-stack darkening)
    let tint_brush = compositor.CreateColorBrushWithColor(recipe.tint_color).ok()?;
    let tint_base: CompositionBrush = tint_brush.cast().ok()?;
    let tint_vis = make_full_sprite(compositor, &tint_base)?;
    children.InsertAtTop(&tint_vis).ok()?;

    // Layer 3: Vertical white volumetric frost scattering veil
    if recipe.frost_veil[0] > 0.002 {
        let veil_stops = [
            (0.0_f32, white_alpha(recipe.frost_veil[0])),
            (0.55, white_alpha(recipe.frost_veil[1])),
            (1.0, white_alpha(recipe.frost_veil[2])),
        ];
        if let Some(veil_brush) = build_linear_gradient_brush(
            compositor,
            Vector2 { X: 0.0, Y: 0.0 },
            Vector2 { X: 0.0, Y: 1.0 },
            &veil_stops,
        ) {
            if let Some(veil_vis) = make_full_sprite(compositor, &veil_brush) {
                let _ = children.InsertAtTop(&veil_vis);
            }
        }
    }

    // Layer 4: 3-ribbon flowing GPU radial Liquid Silk field (accent-synced with Start Menu & MediaBar)
    if recipe.enable_silk {
        if let Ok(cubic) = compositor.CreateCubicBezierEasingFunction(
            Vector2 { X: 0.42, Y: 0.0 },
            Vector2 { X: 0.58, Y: 1.0 },
        ) {
            if let Ok(easing) = cubic.cast::<CompositionEasingFunction>() {
                let pearl_rgb = lifted_pearl_rgb(halo0_rgb, halo1_rgb, light_glass);
                let (p0, p1, p2) = if light_glass {
                    (0.16_f32, 0.13_f32, 0.12_f32)
                } else {
                    (0.22_f32, 0.17_f32, 0.15_f32)
                };
                let ribbons = [
                    (
                        halo0_rgb,
                        p0,
                        Vector2 { X: 0.18, Y: 0.24 },
                        Vector2 { X: 0.78, Y: 0.68 },
                        Vector2 { X: 0.58, Y: 0.42 },
                        14i64,
                    ),
                    (
                        halo1_rgb,
                        p1,
                        Vector2 { X: 0.82, Y: 0.76 },
                        Vector2 { X: 0.28, Y: 0.34 },
                        Vector2 { X: 0.54, Y: 0.46 },
                        19i64,
                    ),
                    (
                        pearl_rgb,
                        p2,
                        Vector2 { X: 0.36, Y: 0.82 },
                        Vector2 { X: 0.72, Y: 0.22 },
                        Vector2 { X: 0.48, Y: 0.36 },
                        23i64,
                    ),
                ];
                for &(rgb, peak, start, end, radius, secs) in &ribbons {
                    if let Some(vis) = build_silk_ribbon_sprite(
                        compositor,
                        &easing,
                        rgb,
                        peak,
                        start,
                        end,
                        radius,
                        secs,
                    ) {
                        let _ = children.InsertAtTop(&vis);
                    }
                }
            }
        }
    }

    // Layer 5: Diagonal white specular sheen (0,0 -> 1,1)
    if recipe.sheen_scale > 0.01 {
        let ss = recipe.sheen_scale;
        let (top_a, mid_a, bot_a) = if light_glass {
            (0.24 * ss, 0.03 * ss, 0.09 * ss)
        } else {
            (0.14 * ss, 0.02 * ss, 0.06 * ss)
        };
        let sheen_stops = [
            (0.0_f32, white_alpha(top_a)),
            (0.45, white_alpha(mid_a)),
            (1.0, white_alpha(bot_a)),
        ];
        if let Some(sheen_brush) = build_linear_gradient_brush(
            compositor,
            Vector2 { X: 0.0, Y: 0.0 },
            Vector2 { X: 1.0, Y: 1.0 },
            &sheen_stops,
        ) {
            if let Some(sheen_vis) = make_full_sprite(compositor, &sheen_brush) {
                let _ = children.InsertAtTop(&sheen_vis);
            }
        }
    }

    clipped.cast().ok()
}

fn build_outer_shadow_visual(
    compositor: &Compositor,
    host_obj: &CompositionObject,
    corner_radius: f32,
    shadow_scale: f32,
    light_glass: bool,
) -> Option<Visual> {
    let shadow_vis = compositor.CreateShapeVisual().ok()?;
    shadow_vis
        .SetRelativeSizeAdjustment(Vector2 { X: 0.0, Y: 0.0 })
        .ok()?;
    shadow_vis
        .SetOffset(Vector3 {
            X: -SHADOW_PAD_DIP,
            Y: -SHADOW_PAD_DIP,
            Z: 0.0,
        })
        .ok()?;
    let pad = Vector2 {
        X: SHADOW_PAD_DIP * 2.0,
        Y: SHADOW_PAD_DIP * 2.0,
    };
    let vis_size_anim = make_size_expr_animation(compositor, host_obj, pad)?;
    shadow_vis
        .StartAnimation(&HSTRING::from("Size"), &vis_size_anim)
        .ok()?;

    let shapes = shadow_vis.Shapes().ok()?;
    let r = corner_radius.max(0.0);
    // Four concentric exterior-only shadow rings overlapping seamlessly beneath the card edge.
    let rings: [(f32, f32, f32, f32); 4] = [
        (12.0, 3.5, 8.0 / 255.0, 5.0 / 255.0),
        (7.5, 2.2, 12.0 / 255.0, 8.0 / 255.0),
        (4.0, 1.2, 17.0 / 255.0, 11.0 / 255.0),
        (1.8, 0.5, 22.0 / 255.0, 14.0 / 255.0),
    ];

    for &(stroke_w, dy, alpha_dark, alpha_light) in &rings {
        let outset = stroke_w * 0.5 + 0.5;
        let base_a = if light_glass { alpha_light } else { alpha_dark };
        let alpha = (base_a * shadow_scale).clamp(0.0, 1.0);
        if alpha <= 0.002 {
            continue;
        }
        let Ok(rr_geo) = compositor.CreateRoundedRectangleGeometry() else {
            continue;
        };
        let _ = rr_geo.SetCornerRadius(Vector2 {
            X: r + outset,
            Y: r + outset,
        });
        let _ = rr_geo.SetOffset(Vector2 {
            X: SHADOW_PAD_DIP - outset,
            Y: SHADOW_PAD_DIP - outset,
        });
        let Some(ring_size_anim) = make_size_expr_animation(
            compositor,
            host_obj,
            Vector2 {
                X: outset * 2.0,
                Y: outset * 2.0 + dy,
            },
        ) else {
            continue;
        };
        if rr_geo
            .StartAnimation(&HSTRING::from("Size"), &ring_size_anim)
            .is_err()
        {
            continue;
        }
        let Ok(geo) = rr_geo.cast::<CompositionGeometry>() else {
            continue;
        };
        let Ok(shape) = compositor.CreateSpriteShapeWithGeometry(&geo) else {
            continue;
        };
        let Ok(brush) = compositor.CreateColorBrushWithColor(black_alpha(alpha)) else {
            continue;
        };
        let Ok(brush_base) = brush.cast::<CompositionBrush>() else {
            continue;
        };
        let _ = shape.SetStrokeThickness(stroke_w);
        let _ = shape.SetStrokeBrush(&brush_base);
        if let Ok(shape_base) = shape.cast::<CompositionShape>() {
            let _ = shapes.Append(&shape_base);
        }
    }

    shadow_vis.cast().ok()
}

fn build_rim_visual(
    compositor: &Compositor,
    host_obj: &CompositionObject,
    corner_radius: f32,
    recipe: &StartModalVisualRecipe,
) -> Option<Visual> {
    let rim_vis = compositor.CreateShapeVisual().ok()?;
    rim_vis
        .SetRelativeSizeAdjustment(Vector2 { X: 0.0, Y: 0.0 })
        .ok()?;
    rim_vis
        .SetOffset(Vector3 {
            X: -1.0,
            Y: -1.0,
            Z: 0.0,
        })
        .ok()?;
    let vis_size_anim =
        make_size_expr_animation(compositor, host_obj, Vector2 { X: 2.0, Y: 2.0 })?;
    rim_vis
        .StartAnimation(&HSTRING::from("Size"), &vis_size_anim)
        .ok()?;

    let rr_rim = compositor.CreateRoundedRectangleGeometry().ok()?;
    let rim_r = (corner_radius.max(0.0) - 0.5).max(1.0);
    rr_rim
        .SetCornerRadius(Vector2 { X: rim_r, Y: rim_r })
        .ok()?;
    rr_rim.SetOffset(Vector2 { X: 1.5, Y: 1.5 }).ok()?;
    let rim_size_anim =
        make_size_expr_animation(compositor, host_obj, Vector2 { X: -1.0, Y: -1.0 })?;
    rr_rim
        .StartAnimation(&HSTRING::from("Size"), &rim_size_anim)
        .ok()?;
    let geo: CompositionGeometry = rr_rim.cast().ok()?;
    let shapes = rim_vis.Shapes().ok()?;

    if recipe.rim_dark_sep > 0.004 {
        if let (Ok(sep_shape), Ok(sep_brush)) = (
            compositor.CreateSpriteShapeWithGeometry(&geo),
            compositor.CreateColorBrushWithColor(black_alpha(recipe.rim_dark_sep)),
        ) {
            if let (Ok(sep_brush_base), Ok(sep_shape_base)) = (
                sep_brush.cast::<CompositionBrush>(),
                sep_shape.cast::<CompositionShape>(),
            ) {
                let _ = sep_shape.SetStrokeThickness(1.25);
                let _ = sep_shape.SetStrokeBrush(&sep_brush_base);
                let _ = shapes.Append(&sep_shape_base);
            }
        }
    }

    let rim_stops = [
        (0.0_f32, white_alpha(recipe.rim_stops[0])),
        (0.55, white_alpha(recipe.rim_stops[1])),
        (1.0, white_alpha(recipe.rim_stops[2])),
    ];
    if let (Ok(rim_shape), Some(rim_brush)) = (
        compositor.CreateSpriteShapeWithGeometry(&geo),
        build_linear_gradient_brush(
            compositor,
            Vector2 { X: 0.0, Y: 0.0 },
            Vector2 { X: 0.0, Y: 1.0 },
            &rim_stops,
        ),
    ) {
        if let Ok(rim_shape_base) = rim_shape.cast::<CompositionShape>() {
            let _ = rim_shape.SetStrokeThickness(recipe.rim_stroke_dip);
            let _ = rim_shape.SetStrokeBrush(&rim_brush);
            let _ = shapes.Append(&rim_shape_base);
        }
    }

    rim_vis.cast().ok()
}

/// Attaches the 7-layer pure-GPU `Windows.UI.Composition` Liquid Glass stack onto `host_insp`,
/// matching PrettyWindows styles and colors while keeping underlying Start Menu controls intact.
pub(super) unsafe fn attach_start_modal_glass_visual(
    host_insp: *mut c_void,
    corner_radius: f32,
    style_id: u8,
    light_glass: bool,
    halo0_rgb: u32,
    halo1_rgb: u32,
) -> bool {
    if host_insp.is_null() {
        return false;
    }
    let Some(ecp) = cached_element_composition_preview() else {
        return false;
    };
    let ui_elem = qi_interface(host_insp, &IID_IUIELEMENT);
    if ui_elem.is_null() {
        return false;
    }
    let ecp_iface = ecp.as_ptr().cast::<RawIElementCompositionPreviewStatics>();
    let mut vis_raw = ComRaw::null();
    if ((*(*ecp_iface).lp_vtbl).get_element_visual)(ecp.as_ptr(), ui_elem.as_ptr(), &mut vis_raw.0)
        .is_err()
        || vis_raw.is_null()
    {
        return false;
    }
    let _ = size_of::<Visual>();
    let host_vis = Visual::from_raw(vis_raw.into_raw());
    let Ok(compositor) = host_vis.Compositor() else {
        return false;
    };
    let Ok(host_obj) = host_vis.cast::<CompositionObject>() else {
        return false;
    };

    let Ok(root) = compositor.CreateContainerVisual() else {
        return false;
    };
    if root
        .SetRelativeSizeAdjustment(Vector2 { X: 1.0, Y: 1.0 })
        .is_err()
    {
        return false;
    }
    let Ok(root_children) = root.Children() else {
        return false;
    };

    let recipe = start_modal_visual_recipe(style_id, light_glass);
    let Some(clipped_vis) = build_clipped_glass_container(
        &compositor,
        &host_obj,
        corner_radius,
        &recipe,
        halo0_rgb,
        halo1_rgb,
        light_glass,
    ) else {
        return false;
    };
    if root_children.InsertAtTop(&clipped_vis).is_err() {
        return false;
    }

    // Place outer shadow after the clipped blur container so CreateBackdropBrush never samples it.
    if let Some(shadow_vis) = build_outer_shadow_visual(
        &compositor,
        &host_obj,
        corner_radius,
        recipe.shadow_scale,
        light_glass,
    ) {
        let _ = root_children.InsertAtTop(&shadow_vis);
    }

    if let Some(rim_vis) = build_rim_visual(&compositor, &host_obj, corner_radius, &recipe) {
        let _ = root_children.InsertAtTop(&rim_vis);
    }

    let Ok(root_vis): Result<Visual> = root.cast() else {
        return false;
    };
    ((*(*ecp_iface).lp_vtbl).set_element_child_visual)(
        ecp.as_ptr(),
        ui_elem.as_ptr(),
        root_vis.as_raw(),
    )
    .is_ok()
}

pub(super) unsafe fn detach_element_child_visual(elem_insp: *mut c_void) {
    if elem_insp.is_null() {
        return;
    }
    let Some(ecp) = cached_element_composition_preview() else {
        return;
    };
    let ui_elem = qi_interface(elem_insp, &IID_IUIELEMENT);
    if ui_elem.is_null() {
        return;
    }
    let ecp_iface = ecp.as_ptr().cast::<RawIElementCompositionPreviewStatics>();
    let _ = ((*(*ecp_iface).lp_vtbl).set_element_child_visual)(
        ecp.as_ptr(),
        ui_elem.as_ptr(),
        null_mut(),
    );
}
