//! Fogbreak Tidal Strata light & dark design tokens, typography, and spacing.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use egui::epaint::text::VariationCoords;
use egui::{Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, FontTweak, Stroke, TextStyle, Visuals};

static IS_DARK: AtomicBool = AtomicBool::new(true);

const fn hex(rgb: u32) -> Color32 {
  let [_, r, g, b] = rgb.to_be_bytes();
  Color32::from_rgb(r, g, b)
}

const fn alpha(rgb: u32, a: u8) -> Color32 {
  let [_, r, g, b] = rgb.to_be_bytes();
  Color32::from_rgba_unmultiplied_const(r, g, b, a)
}

/// Whether the active Tidal Strata palette is dark mode.
pub fn is_dark() -> bool {
  IS_DARK.load(Ordering::Relaxed)
}

/// Updates the active Tidal Strata color mode without rebuilding fonts.
pub fn set_dark(dark: bool) {
  IS_DARK.store(dark, Ordering::Relaxed);
}

/// Primary canvas/feed background (`#111821` dark, `#F7FAFC` light).
pub fn bg() -> Color32 {
  if is_dark() { hex(0x111821) } else { hex(0xF7FAFC) }
}

/// Top bar, modals, and card panel surface (`#1A2431` dark, `#FFFFFF` light).
pub fn panel() -> Color32 {
  if is_dark() { hex(0x1A2431) } else { hex(0xFFFFFF) }
}

/// Elevated interactive surfaces: sidebar, prompt box, toolbar buttons.
pub fn controls() -> Color32 {
  if is_dark() { hex(0x223041) } else { hex(0xEEF4F8) }
}

/// Window/frame chrome behind the main content panel.
pub fn chrome() -> Color32 {
  if is_dark() { hex(0x0C1219) } else { hex(0xE6EEF4) }
}

/// Sunken wells and text inputs.
pub fn sunken() -> Color32 {
  if is_dark() { hex(0x0D141C) } else { hex(0xE8F0F6) }
}

/// Primary body text.
pub fn ink() -> Color32 {
  if is_dark() { hex(0xEAF1F7) } else { hex(0x111821) }
}

/// High-emphasis text.
pub fn ink_strong() -> Color32 {
  if is_dark() { Color32::WHITE } else { hex(0x091017) }
}

/// Secondary text and placeholders.
pub fn muted() -> Color32 {
  if is_dark() { hex(0x9DB0C2) } else { hex(0x4F6375) }
}

/// Idle toolbar icons.
pub fn dim() -> Color32 {
  if is_dark() { alpha(0xEAF1F7, 132) } else { alpha(0x111821, 132) }
}

/// Counters and HUD captions.
pub fn faint() -> Color32 {
  if is_dark() { alpha(0xEAF1F7, 102) } else { alpha(0x111821, 102) }
}

/// Sidebar link ink.
pub fn sidebar_ink() -> Color32 {
  if is_dark() { alpha(0xEAF1F7, 198) } else { alpha(0x111821, 198) }
}

/// Standard 1 px hairline divider.
pub fn line() -> Color32 {
  if is_dark() { hex(0x2C3B4D) } else { hex(0xCBD8E2) }
}

/// Dashed drop target outline.
pub fn line_dashed() -> Color32 {
  if is_dark() { alpha(0xEAF1F7, 72) } else { alpha(0x111821, 72) }
}

/// Outlined buttons and corner marks.
pub fn line_strong() -> Color32 {
  if is_dark() { alpha(0xEAF1F7, 112) } else { alpha(0x111821, 112) }
}

/// Quiet hover/fill wash.
pub fn wash() -> Color32 {
  if is_dark() { alpha(0xFFFFFF, 14) } else { alpha(0x111821, 12) }
}

/// Selected row fill wash.
pub fn wash_selected() -> Color32 {
  if is_dark() { alpha(0x087F9C, 54) } else { alpha(0x087F9C, 34) }
}

/// Hovered interactive row fill wash.
pub fn wash_hover() -> Color32 {
  if is_dark() { alpha(0xFFFFFF, 26) } else { alpha(0x111821, 20) }
}

/// Fogbreak Tidal Strata teal accent (`#087F9C`).
pub fn accent() -> Color32 {
  hex(0x087F9C)
}

/// Accent highlight / progress stroke (`#0A95B8`).
pub fn accent_400() -> Color32 {
  hex(0x0A95B8)
}

/// Accent text and focus rings.
pub fn accent_ink() -> Color32 {
  if is_dark() { hex(0x2BC0E4) } else { hex(0x066882) }
}

/// Inverted primary CTA background.
pub fn invert_bg() -> Color32 {
  if is_dark() { hex(0xEAF1F7) } else { hex(0x111821) }
}

/// Inverted primary CTA foreground text.
pub fn invert_fg() -> Color32 {
  if is_dark() { hex(0x111821) } else { Color32::WHITE }
}

/// Failure text.
pub fn bad() -> Color32 {
  if is_dark() { hex(0xF87171) } else { hex(0xC82333) }
}

/// Destructive button fills.
pub fn danger() -> Color32 {
  hex(0xD33242)
}

/// Highlight badge fill.
pub fn badge_beta() -> Color32 {
  if is_dark() { hex(0xFFC14D) } else { hex(0xC97A00) }
}

/// 3 px: buttons, inputs, menus and the prompt box.
pub const RADIUS: CornerRadius = CornerRadius::same(3);

/// Height of toolbar buttons (`py-1.5` + 14 px text + hairline).
pub const CONTROL_H: f32 = 34.0;
/// The `.hud-label`: 11 px Geist Mono, upper case, 0.15 em apart.
pub const HUD_SIZE: f32 = 11.0;
pub const HUD_SPACING: f32 = HUD_SIZE * 0.15;
/// Section labels and mono buttons are tracked a little tighter (0.12 em).
pub const LABEL_SPACING: f32 = HUD_SIZE * 0.12;

pub fn hairline() -> Stroke {
  Stroke::new(1.0, line())
}

/// `c` at `a` (0–1) of its opacity.
pub fn fade(c: Color32, a: f32) -> Color32 {
  c.gamma_multiply(a.clamp(0.0, 1.0))
}

/// The shadow under menus, modals and toasts.
pub fn shadow() -> egui::epaint::Shadow {
  let alpha = if is_dark() { 140 } else { 42 };
  egui::epaint::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(alpha) }
}

/// Inter 500.
pub fn medium() -> FontFamily {
  FontFamily::Name("medium".into())
}

/// Inter 600.
pub fn semibold() -> FontFamily {
  FontFamily::Name("semibold".into())
}

/// Archivo 620 at 118 % width: `.font-display` headings.
pub fn display() -> FontFamily {
  FontFamily::Name("display".into())
}

/// Geist Mono 500: HUD labels.
pub fn mono() -> FontFamily {
  FontFamily::Name("mono".into())
}

/// Geist Mono 600–700: section labels and mono buttons.
pub fn mono_bold() -> FontFamily {
  FontFamily::Name("mono-bold".into())
}

fn visuals_for(dark: bool) -> Visuals {
  let mut v = if dark { Visuals::dark() } else { Visuals::light() };
  v.dark_mode = dark;
  v.panel_fill = bg();
  v.window_fill = panel();
  v.extreme_bg_color = sunken();
  v.text_edit_bg_color = Some(Color32::TRANSPARENT);
  v.faint_bg_color = controls();
  v.code_bg_color = sunken();
  v.window_stroke = hairline();
  v.window_corner_radius = RADIUS;
  v.menu_corner_radius = RADIUS;
  v.window_shadow = shadow();
  v.popup_shadow = shadow();
  v.override_text_color = Some(ink());
  v.hyperlink_color = accent_ink();
  v.selection.bg_fill = accent();
  v.selection.stroke = Stroke::new(1.0, Color32::WHITE);
  v.text_cursor.stroke = Stroke::new(1.5, ink_strong());
  v.warn_fg_color = badge_beta();
  v.error_fg_color = bad();
  v.striped = false;
  for (w, fill, stroke, fg) in [
    (&mut v.widgets.noninteractive, controls(), line(), muted()),
    (&mut v.widgets.inactive, controls(), line(), ink()),
    (&mut v.widgets.hovered, wash_hover(), line_strong(), ink_strong()),
    (&mut v.widgets.active, wash_hover(), line_strong(), ink_strong()),
    (&mut v.widgets.open, wash_hover(), line_strong(), ink_strong()),
  ] {
    w.bg_fill = fill;
    w.weak_bg_fill = fill;
    w.bg_stroke = Stroke::new(1.0, stroke);
    w.corner_radius = RADIUS;
    w.fg_stroke = Stroke::new(1.0, fg);
    w.expansion = 0.0;
  }
  v
}

/// Installs the fonts, Tidal Strata colours and spacing using the current theme polarity.
pub fn apply(ctx: &egui::Context) {
  apply_theme(ctx, is_dark());
}

/// Installs fonts once and applies Fogbreak Tidal Strata light or dark visuals.
pub fn apply_theme(ctx: &egui::Context, dark: bool) {
  set_dark(dark);
  let fonts_installed_id = egui::Id::new("fogbreak-hub-fonts-installed");
  let installed = ctx.data(|d| d.get_temp::<bool>(fonts_installed_id)).unwrap_or(false);
  if !installed {
    install_fonts(ctx);
    ctx.data_mut(|d| d.insert_temp(fonts_installed_id, true));
  }
  if dark {
    ctx.set_visuals_of(egui::Theme::Dark, visuals_for(true));
    ctx.set_theme(egui::ThemePreference::Dark);
  } else {
    ctx.set_visuals_of(egui::Theme::Light, visuals_for(false));
    ctx.set_theme(egui::ThemePreference::Light);
  }
  ctx.all_styles_mut(|style| {
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(12.0, 6.0);
    style.spacing.interact_size.y = CONTROL_H;
    style.spacing.menu_margin = egui::Margin::same(4);
    style.spacing.window_margin = egui::Margin::same(16);
    style.spacing.scroll = egui::style::ScrollStyle { bar_width: 8.0, floating: true, ..egui::style::ScrollStyle::thin() };
    style.text_styles = [
      (TextStyle::Heading, FontId::new(24.0, display())),
      (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
      (TextStyle::Button, FontId::new(14.0, medium())),
      (TextStyle::Small, FontId::new(12.0, FontFamily::Proportional)),
      (TextStyle::Monospace, FontId::new(13.0, FontFamily::Monospace)),
    ]
    .into();
  });
}

/// A font registered under `name`: its file, its variation coordinates and its family.
type Face<'a> = (&'a str, &'static [u8], &'a [([u8; 4], f32)], FontFamily);

/// Registers the three variable fonts, each at the weights (and width) the workspace uses.
fn install_fonts(ctx: &egui::Context) {
  const INTER: &[u8] = include_bytes!("../assets/fonts/Inter-Variable.ttf");
  const ARCHIVO: &[u8] = include_bytes!("../assets/fonts/Archivo-Variable.ttf");
  const GEIST_MONO: &[u8] = include_bytes!("../assets/fonts/GeistMono-Variable.ttf");
  let mut fonts = FontDefinitions::default();
  // egui's own fonts stay behind ours, for symbols and emoji ours don't have.
  let fallback = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
  let faces: [Face<'_>; 7] = [
    ("Inter", INTER, &[(*b"wght", 400.0), (*b"opsz", 14.0)], FontFamily::Proportional),
    ("Inter-Medium", INTER, &[(*b"wght", 500.0), (*b"opsz", 14.0)], medium()),
    ("Inter-SemiBold", INTER, &[(*b"wght", 600.0), (*b"opsz", 14.0)], semibold()),
    ("Archivo-Display", ARCHIVO, &[(*b"wght", 620.0), (*b"wdth", 118.0)], display()),
    ("GeistMono", GEIST_MONO, &[(*b"wght", 400.0)], FontFamily::Monospace),
    ("GeistMono-Medium", GEIST_MONO, &[(*b"wght", 500.0)], mono()),
    ("GeistMono-SemiBold", GEIST_MONO, &[(*b"wght", 650.0)], mono_bold()),
  ];
  for (name, bytes, coords, family) in faces {
    let tweak = FontTweak { coords: VariationCoords::new(coords.iter().copied()), ..Default::default() };
    fonts.font_data.insert(name.to_owned(), Arc::new(FontData::from_static(bytes).tweak(tweak)));
    let mut stack = vec![name.to_owned()];
    stack.extend(fallback.iter().cloned());
    fonts.families.insert(family, stack);
  }
  ctx.set_fonts(fonts);
}
