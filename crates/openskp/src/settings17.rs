//! The 2017 settings records (SKP_FORMAT §10.6–§10.10) decoded into the
//! typed structs of [`crate::settings`]. These are signature scans over
//! the archive bytes, like the extractors in [`crate::extract`]; the
//! text, dimension and font entities come from the continuous walk's map
//! when the caller has one.

use crate::carchive::{Child, Slot};
use crate::entity::Entity;
use crate::extract::{
    camera_at, f64le, mfc_str_at, mfc_str_end_at, u16le, u32le, viewpage_tag_before,
};
use crate::settings::*;
use crate::INCH;

/// Everything this module decodes.
#[derive(Default)]
pub(crate) struct Settings17 {
    pub camera: Option<Camera>,
    pub rendering: Option<RenderingOptions>,
    pub shadows: Option<ShadowInfo>,
    pub scenes: Vec<Scene>,
    pub styles: Vec<Style>,
    pub active_style: Option<usize>,
    pub watermarks: Vec<Watermark>,
    pub fonts: Vec<Font>,
    pub texts: Vec<Text>,
    pub dimensions: Vec<Dimension>,
    pub axes: Option<Axes>,
    pub text_defaults: Option<TextDefaults>,
    pub dimension_defaults: Option<DimensionDefaults>,
    pub anti_aliased_textures: Option<bool>,
    /// Map slot → index into `fonts`, for resolving font references.
    font_index: std::collections::HashMap<usize, usize>,
}

pub(crate) fn read(d: &[u8], map: Option<&[Slot]>, layer_slots: &[usize]) -> Settings17 {
    let mut s = Settings17::default();
    let mut displayed = false;
    if let Some((camera, ro, base)) = document_view(d) {
        displayed = d.get(base + 27).is_some_and(|&b| b != 0);
        s.camera = Some(camera);
        s.rendering = Some(ro);
    }
    let style_recs = style_records(d);
    let (styles, active, current) = styles_of(&style_recs, &watermark_bodies(d));
    s.styles = styles;
    s.active_style = active;
    s.watermarks = current;
    s.shadows = shadow_records(d)
        .first()
        .and_then(|&i| shadow_at(d, i))
        .map(|(mut sh, _)| {
            sh.displayed = displayed;
            sh
        });
    // The byte after the style manager's unsaved-changes flag and eight
    // zero bytes: anti-aliased textures (Model Info ▸ Rendering).
    s.anti_aliased_textures = style_recs
        .last()
        .and_then(|r| d.get(r.end + 12))
        .map(|&b| b != 0);
    let found = scenes(d, &style_recs, &s.styles, displayed, map, layer_slots);
    let last_scene_end = found.last().map(|(_, end)| *end);
    s.scenes = found.into_iter().map(|(sc, _)| sc).collect();
    if let Some(map) = map {
        annotations(map, &mut s);
    }
    if let Some(&i) = shadow_records(d).first() {
        if let Some((_, end)) = shadow_at(d, i) {
            document_tail(d, end, last_scene_end, map, &mut s);
        }
    }
    s
}

/// After the document shadow record (§10.9): a u16, the page count, the
/// pages, a reference to the selected page, the model axes (110 bytes),
/// 70 zero bytes, then the annotation defaults (§10.11): a font object,
/// the dimension defaults, a font reference, the text defaults, a font
/// reference.
fn document_tail(
    d: &[u8],
    shadow_end: usize,
    last_scene_end: Option<usize>,
    map: Option<&[Slot]>,
    s: &mut Settings17,
) -> Option<()> {
    let n = u32le(d, shadow_end + 2)?;
    let after_pages = if n == 0 {
        shadow_end + 6
    } else {
        last_scene_end?
    };
    let (_, ax) = object_ref(d, after_pages)?;
    let v = |o: usize| -> Option<[f64; 3]> {
        Some([f64le(d, o)?, f64le(d, o + 8)?, f64le(d, o + 16)?])
    };
    let (x, y, z) = (v(ax + 37)?, v(ax + 61)?, v(ax + 85)?);
    let unit = |a: [f64; 3]| ((a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt() - 1.0).abs() < 1e-6;
    if !(unit(x) && unit(y) && unit(z)) {
        return None;
    }
    s.axes = Some(Axes {
        origin_m: m3(v(ax + 13)?),
        x,
        y,
        z,
    });
    let b0 = ax + 110 + 70;
    let (dim_font, b) = font_ref(d, b0, map, s)?;
    let rgba = |o: usize| -> Option<[u8; 4]> { d.get(o..o + 4)?.try_into().ok() };
    let dims = DimensionDefaults {
        font: dim_font,
        aligned: *d.get(b)? != 0,
        arrow: u32le(d, b + 14)?,
        text_position: u32le(d, b + 54)?,
        color: rgba(b + 46)?,
    };
    let (font, t) = font_ref(d, b + 61, map, s)?;
    let (screen_font, _) = font_ref(d, t + 22, map, s)?;
    s.text_defaults = Some(TextDefaults {
        font,
        screen_font,
        arrow: u32le(d, t + 9)?,
        leader_text_color: rgba(t + 14)?,
        screen_text_color: rgba(t + 18)?,
    });
    s.dimension_defaults = Some(dims);
    Some(())
}

/// A font object reference at `at`: an inline CSkFont (added to the font
/// list) or a reference to one the map holds. Returns the font index and
/// the end offset.
fn font_ref(
    d: &[u8],
    at: usize,
    map: Option<&[Slot]>,
    s: &mut Settings17,
) -> Option<(Option<usize>, usize)> {
    let tag = u16le(d, at)?;
    let (new, body, slot) = match tag {
        0 => return Some((None, at + 2)),
        0xFFFF => (true, at + 6 + u16le(d, at + 4)? as usize, None),
        0x7FFF => {
            let v = u32le(d, at + 2)?;
            (
                v & 0x8000_0000 != 0,
                at + 6,
                Some((v & 0x7FFF_FFFF) as usize),
            )
        }
        t => (t & 0x8000 != 0, at + 2, Some((t & 0x7FFF) as usize)),
    };
    if !new {
        let idx = slot
            .and_then(|sl| s.font_index.get(&sl).copied())
            .or_else(|| {
                let Some(Slot::Object(Entity::Font { name, .. })) = map.and_then(|m| m.get(slot?))
                else {
                    return None;
                };
                s.fonts.iter().position(|f| &f.family == name)
            });
        return Some((idx, body));
    }
    // inline: 3 zero bytes, the name, then bold, italic, u32 size, u8, f64
    let name = mfc_str_at(d, body + 3)?;
    let j = mfc_str_end_at(d, body + 3)?;
    let t = d.get(j..j + 15)?;
    s.fonts.push(Font {
        family: name,
        bold: t[0] != 0,
        italic: t[1] != 0,
        size_pt: u32::from_le_bytes([t[2], t[3], t[4], t[5]]),
    });
    Some((Some(s.fonts.len() - 1), j + 15))
}

/// The document camera, its rendering options and the options block's
/// base offset (§10.6, §10.7).
fn document_view(d: &[u8]) -> Option<(Camera, RenderingOptions, usize)> {
    let i = d.windows(7).position(|w| w == b"CCamera")?;
    let body = i + 7;
    let after = body + 137;
    let desc = mfc_str_at(d, after + 2)?;
    let tail = mfc_str_end_at(d, after + 2)?;
    let base = tail + 33 + 3;
    Some((
        Camera::from_2017(d, body, tail, desc)?,
        RenderingOptions::from_2017(d, base)?,
        base,
    ))
}

/// Offsets of every §10.9 shadow record: three zero bytes, a plausible
/// UNIX time, a byte, then two strings. The first is the document's.
fn shadow_records(d: &[u8]) -> Vec<usize> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 12 <= d.len() {
        if d[i..i + 3] == [0, 0, 0]
            && d[i + 8..i + 11] == [0xff, 0xfe, 0xff]
            && u32le(d, i + 3).is_some_and(|t| (1_000_000_000..3_000_000_000).contains(&t))
            && shadow_at(d, i).is_some()
        {
            out.push(i);
        }
        i += 1;
    }
    out
}

/// The shadow record at `i` and its end (§10.9). `displayed` is not part
/// of the record; the caller sets it.
fn shadow_at(d: &[u8], i: usize) -> Option<(ShadowInfo, usize)> {
    let time = u32le(d, i + 3)?;
    let city = mfc_str_at(d, i + 8)?;
    let j = mfc_str_end_at(d, i + 8)?;
    let country = mfc_str_at(d, j)?;
    let j = mfc_str_end_at(d, j)?;
    let longitude = f64le(d, j)?;
    let latitude = f64le(d, j + 8)?;
    let tz_offset_h = f64le(d, j + 16)?;
    if !(-181.0..=181.0).contains(&longitude) || !(-91.0..=91.0).contains(&latitude) {
        return None;
    }
    let k = j + 48; // after the north vector
    let b = d.get(k..k + 14)?;
    Some((
        ShadowInfo {
            time,
            city,
            country,
            longitude,
            latitude,
            tz_offset_h,
            displayed: false,
            on_faces: b[1] != 0,
            on_ground: b[2] != 0,
            from_edges: b[3] != 0,
            light: u32::from_le_bytes([b[4], b[5], b[6], b[7]]),
            dark: u32::from_le_bytes([b[8], b[9], b[10], b[11]]),
            use_sun_for_shading: b[12] != 0,
        },
        k + 14,
    ))
}

/// A CSkpStyle record (§10.8) as found in the file.
struct StyleRec {
    at: usize,
    end: usize,
    guid: String,
    name: String,
    description: String,
    settings: RenderingOptions,
    /// The item-5001 entries: an inline watermark, a back-reference, or the
    /// null that separates backgrounds from overlays.
    list: Vec<WmEntry>,
}

enum WmEntry {
    Inline(Watermark),
    Back,
    Model,
}

/// Skip an object reference at `at` (§3.1): returns `(is_new_object,
/// body start)`; a new class record is skipped with its name.
fn object_ref(d: &[u8], at: usize) -> Option<(bool, usize)> {
    let tag = u16le(d, at)?;
    Some(match tag {
        0 => (false, at + 2),
        0xFFFF => {
            let n = u16le(d, at + 4)? as usize;
            (true, at + 6 + n)
        }
        0x7FFF => {
            let v = u32le(d, at + 2)?;
            (v & 0x8000_0000 != 0, at + 6)
        }
        t => (t & 0x8000 != 0, at + 2),
    })
}

/// Every CWatermark body in the file, in order (§10.8): the pre-model
/// manager serializes each once; style lists refer back to them.
fn watermark_bodies(d: &[u8]) -> Vec<(usize, Watermark)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 8 <= d.len() {
        if d[i..i + 7] == [0, 0, 0, 0, 0xff, 0xfe, 0xff] {
            if let Some((w, end)) = watermark_at(d, i, false) {
                out.push((i, w));
                i = end;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// A CWatermark body at `i` (§10.8) and its end.
fn watermark_at(d: &[u8], i: usize, background: bool) -> Option<(Watermark, usize)> {
    let source_file = mfc_str_at(d, i + 4)?;
    let j = mfc_str_end_at(d, i + 4)?;
    let time = u32le(d, j)?;
    let position = u32le(d, j + 4)?;
    let b = d.get(j + 8..j + 13)?;
    let blend = f64le(d, j + 13)?;
    let scale = f64le(d, j + 21)?;
    if source_file.is_empty()
        || !(1_000_000_000..3_000_000_000).contains(&time)
        || position > 8
        || b.iter().any(|&x| x > 1)
        || !(0.0..=1.0).contains(&blend)
        || !(0.0..=1.0).contains(&scale)
    {
        return None;
    }
    let name = mfc_str_at(d, j + 29)?;
    let k = mfc_str_end_at(d, j + 29)?;
    let (new, body) = object_ref(d, k)?;
    let end = if new {
        body + 8 + u32le(d, body + 4)? as usize
    } else {
        body
    };
    // The 2017 record keeps the aspect-ratio byte set while the wizard
    // hides the control; the flag only means anything for a stretched
    // watermark, as the post-2017 conversion also decides (§10.8).
    let stretched = b[1] != 0;
    Some((
        Watermark {
            name,
            source_file,
            background,
            tiled: b[0] != 0,
            stretched,
            keep_aspect_ratio: stretched && b[2] != 0,
            position,
            mask: b[4] != 0,
            blend,
            scale,
        },
        end,
    ))
}

/// Every CSkpStyle record in the file, in order (§10.8).
fn style_records(d: &[u8]) -> Vec<StyleRec> {
    let mut out = Vec::new();
    let mut i = 0;
    // Preamble: null attribute pointer + pid field (mask 0 on authored
    // files; third-party styles carry pids), then GUID, empty string,
    // u32 3, name.
    while i + 30 <= d.len() {
        let k = (d[i + 2] & 0x0f).count_ones() as usize;
        if d[i] == 0
            && d[i + 1] == 0
            && d[i + 2] <= 0x0f
            && d.get(i + 19 + k..i + 27 + k)
                == Some(&[0xff, 0xfe, 0xff, 0x00, 0x03, 0x00, 0x00, 0x00][..])
            && d.get(i + 27 + k..i + 30 + k) == Some(&[0xff, 0xfe, 0xff][..])
        {
            if let Some(r) = style_at(d, i) {
                i = r.end;
                out.push(r);
                continue;
            }
        }
        i += 1;
    }
    out
}

fn style_at(d: &[u8], i: usize) -> Option<StyleRec> {
    let k = (d.get(i + 2)? & 0x0f).count_ones() as usize;
    let guid: String = d
        .get(i + 3 + k..i + 19 + k)?
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let name = mfc_str_at(d, i + 27 + k)?;
    let j = mfc_str_end_at(d, i + 27 + k)?;
    let description = mfc_str_at(d, j)?;
    let mut j = mfc_str_end_at(d, j)?;
    let n = u32le(d, j)?;
    if !(10..200).contains(&n) {
        return None;
    }
    j += 4;
    let mut settings = RenderingOptions::default();
    let mut list = Vec::new();
    for _ in 0..n {
        let key = u32le(d, j)?;
        let count = u32le(d, j + 4)? as usize;
        j += 8;
        if key == 5001 {
            let mut background = true;
            for _ in 0..count {
                let (new, body) = object_ref(d, j)?;
                if u16le(d, j)? == 0 {
                    background = false;
                    list.push(WmEntry::Model);
                    j = body;
                } else if new {
                    let (w, end) = watermark_at(d, body, background)?;
                    list.push(WmEntry::Inline(w));
                    j = end;
                } else {
                    list.push(WmEntry::Back);
                    j = body;
                }
            }
            continue;
        }
        for _ in 0..count {
            let ty = u32le(d, j)?;
            let (kind, size) = match ty {
                1 => (RoKind::Bool, 1),
                4 => (RoKind::U32, 4),
                7 => (RoKind::F64, 8),
                _ => return None,
            };
            let raw = d.get(j + 4..j + 4 + size)?;
            let v = match RO_FIELDS.iter().find(|f| f.item == Some(key)) {
                Some(f) if f.kind == RoKind::Rgba => RoValue::Rgba(raw.try_into().ok()?),
                _ => RoValue::decode(kind, raw)?,
            };
            settings.set_item(key, v);
            j += 4 + size;
        }
    }
    Some(StyleRec {
        at: i,
        end: j,
        guid,
        name,
        description,
        settings,
        list,
    })
}

/// The saved styles, the active style's index and the current settings'
/// watermarks. The last record holds the current settings (its GUID names
/// the active style); every other record is a saved style, listed once by
/// GUID.
fn styles_of(
    recs: &[StyleRec],
    bodies: &[(usize, Watermark)],
) -> (Vec<Style>, Option<usize>, Vec<Watermark>) {
    let Some((current, saved)) = recs.split_last() else {
        return (Vec::new(), None, Vec::new());
    };
    // Watermarks referenced rather than inlined resolve, in order, to the
    // watermark bodies outside the record itself.
    let resolve = |r: &StyleRec| -> Vec<Watermark> {
        let others: Vec<&Watermark> = bodies
            .iter()
            .filter(|(at, _)| !(r.at..r.end).contains(at))
            .map(|(_, w)| w)
            .collect();
        let mut k = 0;
        let mut background = true;
        let mut out = Vec::new();
        for e in &r.list {
            match e {
                WmEntry::Inline(w) => out.push(w.clone()),
                WmEntry::Model => background = false,
                WmEntry::Back => {
                    if let Some(w) = others.get(k) {
                        let mut w = (*w).clone();
                        w.background = background;
                        out.push(w);
                    }
                    k += 1;
                }
            }
        }
        out
    };
    let mut styles: Vec<Style> = Vec::new();
    for r in saved {
        if styles.iter().any(|s| s.guid == r.guid) {
            continue;
        }
        styles.push(Style {
            name: r.name.clone(),
            description: r.description.clone(),
            guid: r.guid.clone(),
            settings: r.settings.clone(),
            watermarks: resolve(r),
        });
    }
    let active = styles.iter().position(|s| s.guid == current.guid);
    let current_wm = resolve(current);
    (styles, active, current_wm)
}

/// Read a reference list (`u32 n`, n object references) and return the
/// referenced entities' persistent ids (when a map is available) and the
/// end offset.
fn ref_list(d: &[u8], at: usize, map: Option<&[Slot]>) -> Option<(Vec<u32>, usize)> {
    let (slots, end) = ref_slots(d, at)?;
    let ids = slots
        .iter()
        .filter_map(|&slot| match map?.get(slot)? {
            Slot::Object(e) => e.pid(),
            _ => None,
        })
        .collect();
    Some((ids, end))
}

/// A counted list of object references as global map slots.
fn ref_slots(d: &[u8], at: usize) -> Option<(Vec<usize>, usize)> {
    let n = u32le(d, at)?;
    if n > 0xFFFF {
        return None;
    }
    let mut i = at + 4;
    let mut slots = Vec::with_capacity(n as usize);
    for _ in 0..n {
        let tag = u16le(d, i)?;
        let (slot, next) = if tag == 0x7FFF {
            ((u32le(d, i + 2)? & 0x7FFF_FFFF) as usize, i + 6)
        } else {
            ((tag & 0x7FFF) as usize, i + 2)
        };
        slots.push(slot);
        i = next;
    }
    Some((slots, i))
}

/// Every CViewPage record (§10.10).
fn scenes(
    d: &[u8],
    style_recs: &[StyleRec],
    styles: &[Style],
    displayed: bool,
    map: Option<&[Slot]>,
    layer_slots: &[usize],
) -> Vec<(Scene, usize)> {
    let mut out = Vec::new();
    // The layer-list objects, list order, are `Model::layers` (model.rs);
    // each definition's inline Layer0 copy is a separate map object.
    // A page preamble: null attribute pointer, pid mask, pid bytes (§5.1;
    // mask 0 on authored files, a pid on third-party models), then the
    // name string marker.
    let mut i = 0;
    while i + 6 <= d.len() {
        let k = (d[i + 2] & 0x0f).count_ones() as usize;
        if d[i] == 0
            && d[i + 1] == 0
            && d[i + 2] <= 0x0f
            && d.get(i + 3 + k..i + 6 + k) == Some(&[0xff, 0xfe, 0xff][..])
            && viewpage_tag_before(d, i)
        {
            if let Some((s, end)) = scene_at(d, i, style_recs, styles, displayed, map, layer_slots)
            {
                out.push((s, end));
                i = end;
                continue;
            }
        }
        i += 1;
    }
    out
}

fn scene_at(
    d: &[u8],
    i: usize,
    style_recs: &[StyleRec],
    styles: &[Style],
    doc_displayed: bool,
    map: Option<&[Slot]>,
    layer_slots: &[usize],
) -> Option<(Scene, usize)> {
    let pre = i + 3 + (d.get(i + 2)? & 0x0f).count_ones() as usize;
    let name = mfc_str_at(d, pre)?;
    let j = mfc_str_end_at(d, pre)?;
    let description = mfc_str_at(d, j)?;
    let mut j = mfc_str_end_at(d, j)?;
    let flags = u32le(d, j)?;
    if flags > 0xFFFF {
        return None;
    }
    j += 4;
    let saved = SceneProperties(flags);
    let mut camera = None;
    if flags & 1 != 0 {
        if !camera_at(d, j) {
            return None;
        }
        let (_, body) = object_ref(d, j)?;
        let after = body + 137;
        let desc = mfc_str_at(d, after + 2)?;
        let tail = mfc_str_end_at(d, after + 2)?;
        camera = Some(Camera::from_2017(d, body, tail, desc)?);
        j = tail + 33;
    }
    let mut rendering = None;
    let mut style = None;
    let mut scene_displayed = None;
    if flags & 2 != 0 {
        if d.get(j..j + 3)? != [0, 0, 0] {
            return None;
        }
        rendering = Some(RenderingOptions::from_2017(d, j + 3)?);
        scene_displayed = d.get(j + 3 + 27).map(|&b| b != 0);
        j += 3 + 206;
        let (new, body) = object_ref(d, j)?;
        if new {
            let r = style_recs.iter().find(|r| r.at == body)?;
            style = Some(r.name.clone());
            j = r.end;
        } else {
            if styles.len() == 1 {
                style = Some(styles[0].name.clone());
            }
            j = body;
        }
    }
    let mut shadows = None;
    if flags & 4 != 0 {
        let (mut sh, end) = shadow_at(d, j)?;
        sh.displayed = scene_displayed.unwrap_or(doc_displayed);
        shadows = Some(sh);
        j = end;
    }
    let mut axes = None;
    if flags & 8 != 0 {
        let v = |o: usize| -> Option<[f64; 3]> {
            Some([f64le(d, o)?, f64le(d, o + 8)?, f64le(d, o + 16)?])
        };
        let o = v(j + 13)?;
        axes = Some(Axes {
            origin_m: [o[0] / INCH, o[1] / INCH, o[2] / INCH],
            x: v(j + 37)?,
            y: v(j + 61)?,
            z: v(j + 85)?,
        });
        j += 110;
    }
    let mut hidden_entities = Vec::new();
    let mut hidden_layers = Vec::new();
    let mut active_section_planes = Vec::new();
    if flags & 16 != 0 {
        let (ids, end) = ref_list(d, j, map)?;
        hidden_entities = ids;
        j = end;
    }
    if flags & 32 != 0 {
        // Layers carry no pid on authored files: resolve by map slot.
        let (slots, end) = ref_slots(d, j)?;
        hidden_layers = slots
            .iter()
            .filter_map(|s| layer_slots.iter().position(|q| q == s))
            .collect();
        j = end;
    }
    if flags & 64 != 0 {
        let (ids, end) = ref_list(d, j, map)?;
        active_section_planes = ids;
        j = end;
    }
    let anim = *d.get(j)?;
    if anim > 1 {
        return None;
    }
    // Thumbnail (§10.10): `00 00` none; `01 01` then u8 (1), the PNG
    // length and the PNG; `01 00` then an object pointer to a thumbnail
    // stored earlier (a third-party house).
    let has_thumb = *d.get(j + 19)?;
    let mut end = j + 21;
    if has_thumb == 1 {
        end = match *d.get(j + 20)? {
            1 => j + 26 + u32le(d, j + 22)? as usize,
            _ => object_ref(d, j + 21)?.1,
        };
    }
    Some((
        Scene {
            name,
            description,
            saved,
            camera,
            rendering,
            style,
            shadows,
            axes,
            hidden_entities,
            active_section_planes,
            hidden_layers,
            in_animation: anim != 0,
        },
        end,
    ))
}

fn m3(v: [f64; 3]) -> [f64; 3] {
    [v[0] / INCH, v[1] / INCH, v[2] / INCH]
}

/// Fonts, texts and dimensions from the continuous walk's map.
fn annotations(map: &[Slot], s: &mut Settings17) {
    let mut font_index = std::collections::HashMap::new();
    for (slot, e) in map.iter().enumerate() {
        if let Slot::Object(Entity::Font {
            name,
            bold,
            italic,
            size_pt,
        }) = e
        {
            font_index.insert(slot, s.fonts.len());
            s.fonts.push(Font {
                family: name.clone(),
                bold: *bold,
                italic: *italic,
                size_pt: *size_pt,
            });
        }
    }
    s.font_index = font_index.clone();
    let font_of = |c: &Child| match c {
        Child::Obj(i) => font_index.get(i).copied(),
        _ => None,
    };
    let anchor = |kind: u32, p: [f64; 3], entity: &Child| Anchor {
        kind,
        point_m: if kind == 5 {
            [p[0], p[1] / INCH, p[2] / INCH]
        } else {
            m3(p)
        },
        entity: match entity {
            Child::Obj(slot) => match map.get(*slot) {
                Some(Slot::Object(e)) => e.pid(),
                _ => None,
            },
            _ => None,
        },
    };
    for e in map {
        match e {
            Slot::Object(Entity::Text { pid, body: t }) => {
                let leader_kind = match t.leader {
                    1 => Leader::ViewBased,
                    2 => Leader::Pushpin,
                    _ => Leader::None,
                };
                s.texts.push(Text {
                    pid: *pid,
                    content: t.content.clone(),
                    screen_position: t.screen_position,
                    anchor: anchor(t.anchor_kind, t.anchor_point, &t.anchor_entity),
                    leader_offset: if leader_kind == Leader::Pushpin {
                        m3(t.leader_offset)
                    } else {
                        t.leader_offset
                    },
                    leader: leader_kind,
                    arrow: t.arrow,
                    font: font_of(&t.font),
                });
            }
            Slot::Object(Entity::Dimension { pid, body: dm }) => {
                s.dimensions.push(Dimension {
                    pid: *pid,
                    text_override: dm.text_override.clone(),
                    start: anchor(dm.anchors[0].0, dm.anchors[0].1, &dm.anchors[0].2),
                    end: anchor(dm.anchors[1].0, dm.anchors[1].1, &dm.anchors[1].2),
                    normal: dm.normal,
                    x_axis: dm.x_axis,
                    offset_m: dm.offset / INCH,
                    text_position: dm.text_position,
                    aligned: dm.aligned,
                    font: font_of(&dm.font),
                });
            }
            _ => {}
        }
    }
}
