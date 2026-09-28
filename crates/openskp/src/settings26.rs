//! The post-2017 settings records (SKP_FORMAT §16.9–§16.15) decoded into
//! the typed structs of [`crate::settings`].

use std::collections::HashMap;

use crate::model::Error;
use crate::read26::{entity_id, f64s, text, uint, xml_attrs, Node, Rec};
use crate::settings::*;
use crate::zip::Archive;
use crate::INCH;

fn m3(v: [f64; 3]) -> [f64; 3] {
    [v[0] / INCH, v[1] / INCH, v[2] / INCH]
}

fn f64_at(r: Option<Rec<'_>>) -> Option<f64> {
    let b = r?.data;
    (b.len() == 8).then(|| f64::from_le_bytes(b.try_into().unwrap()))
}

fn flag(r: Option<Rec<'_>>) -> bool {
    uint(r).unwrap_or(0) != 0
}

/// A packed id list (`<width:u8> <id>`, §16.2) as ids.
fn ids(r: Option<Rec<'_>>) -> Vec<u32> {
    let mut out = Vec::new();
    let Some(b) = r.map(|r| r.data) else {
        return out;
    };
    let mut o = 0;
    while o < b.len() {
        let w = b[o] as usize;
        let Some(v) = b.get(o + 1..o + 1 + w) else {
            break;
        };
        out.push(v.iter().rev().fold(0u32, |a, &x| (a << 8) | x as u32));
        o += 1 + w;
    }
    out
}

/// A `0x34bc` camera record (§16.9).
pub(crate) fn camera(n: &Node<'_>) -> Result<Camera, Error> {
    Ok(Camera {
        eye_m: m3(f64s::<3>(n.get(0x34BD), n.at)?),
        target_m: m3(f64s::<3>(n.get(0x34BE), n.at)?),
        up: f64s::<3>(n.get(0x34BF), n.at)?,
        perspective: flag(n.get(0x34C2)),
        fov_deg: f64_at(n.get(0x34C4)).unwrap_or(0.0),
        parallel_height_m: f64_at(n.get(0x34C3)).unwrap_or(0.0) / INCH,
        two_point_perspective: flag(n.get(0x34CA)),
        description: String::new(),
    })
}

/// A `0x733c` rendering-options record (§16.10).
pub(crate) fn rendering(n: &Node<'_>) -> RenderingOptions {
    let mut ro = RenderingOptions::default();
    for r in &n.kids {
        ro.set_record(r.tag, r.data);
    }
    ro
}

/// A `0x6590` shadow-info record (§16.11).
pub(crate) fn shadow(n: &Node<'_>) -> Result<ShadowInfo, Error> {
    Ok(ShadowInfo {
        time: uint(n.get(0x6591)).unwrap_or(0),
        city: text(n.get(0x6593)),
        country: text(n.get(0x6594)),
        longitude: f64_at(n.get(0x6595)).unwrap_or(0.0),
        latitude: f64_at(n.get(0x6596)).unwrap_or(0.0),
        tz_offset_h: f64_at(n.get(0x6597)).unwrap_or(0.0),
        displayed: flag(n.get(0x6599)),
        on_faces: flag(n.get(0x659B)),
        on_ground: flag(n.get(0x659C)),
        from_edges: flag(n.get(0x659D)),
        light: uint(n.get(0x659E)).unwrap_or(0),
        dark: uint(n.get(0x659F)).unwrap_or(0),
        use_sun_for_shading: flag(n.get(0x65A0)),
    })
}

/// The items of a `style.xml`: `(item id, variant type, text)` for the
/// scalar items, and the watermarks of item 5001.
fn style_document(xml: &str) -> (Vec<(u32, u32, String)>, Vec<Watermark>) {
    let mut items = Vec::new();
    let mut watermarks = Vec::new();
    let mut rest = xml;
    while let Some(i) = rest.find("<sty:item id=\"") {
        rest = &rest[i + 14..];
        let Some(q) = rest.find('"') else { break };
        let Ok(id) = rest[..q].parse::<u32>() else {
            break;
        };
        let Some(v) = rest.find("<t:variant type=\"") else {
            break;
        };
        let after = &rest[v + 17..];
        let Some(q2) = after.find('"') else { break };
        let Ok(ty) = after[..q2].parse::<u32>() else {
            break;
        };
        let Some(gt) = after.find('>') else { break };
        let body = &after[gt + 1..];
        let end = body.find("</sty:item>").unwrap_or(body.len());
        if ty == 13 {
            watermarks = screen_images(&body[..end]);
        } else {
            let value = body[..end.min(body.find('<').unwrap_or(end))]
                .trim()
                .to_string();
            items.push((id, ty, value));
        }
        rest = &body[end..];
    }
    (items, watermarks)
}

/// The `<screenimage>` entries of a watermark list, in order, without the
/// `<MODEL SPACE>` separator (§16.14).
fn screen_images(xml: &str) -> Vec<Watermark> {
    let mut out = Vec::new();
    let mut rest = xml;
    let mut background = true;
    while let Some(i) = rest.find(":screenimage ") {
        let Some(a) = xml_attrs(&rest[i..], ":screenimage ") else {
            break;
        };
        let name = a.get("name").cloned().unwrap_or_default();
        if name == "<MODEL SPACE>" {
            background = false;
        } else {
            let b = |k: &str| a.get(k).map(|v| v.trim() != "0").unwrap_or(false);
            let f = |k: &str| {
                a.get(k)
                    .and_then(|v| v.trim().parse::<f64>().ok())
                    .unwrap_or(0.0)
            };
            out.push(Watermark {
                name,
                source_file: a.get("info_filename").cloned().unwrap_or_default(),
                background,
                tiled: b("tiled"),
                stretched: b("stretched"),
                keep_aspect_ratio: b("maintainAR"),
                position: a
                    .get("position")
                    .and_then(|v| v.trim().parse().ok())
                    .unwrap_or(0),
                mask: b("intensForAlpha"),
                blend: f("alphaScale"),
                scale: f("scale"),
            });
        }
        rest = &rest[i + 13..];
    }
    out
}

/// A style's settings from its `styles/<name>/style.xml`.
fn style_settings(zip: &Archive<'_>, name: &str) -> (RenderingOptions, Vec<Watermark>) {
    let mut ro = RenderingOptions::default();
    let Ok(Some(xml)) = zip.read(&format!("styles/{name}/style.xml")) else {
        return (ro, Vec::new());
    };
    let xml = String::from_utf8_lossy(&xml);
    let (items, watermarks) = style_document(&xml);
    for (id, ty, value) in items {
        let kind = match ty {
            1 => RoKind::Bool,
            4 => RoKind::U32,
            7 => RoKind::F64,
            _ => continue,
        };
        if let Some(v) = RoValue::parse_item(kind, &value) {
            // colours travel as signed integers under type 4
            let v = match (v, RO_FIELDS.iter().find(|f| f.item == Some(id))) {
                (RoValue::U32(x), Some(f)) if f.kind == RoKind::Rgba => {
                    RoValue::Rgba(x.to_le_bytes())
                }
                _ => v,
            };
            ro.set_item(id, v);
        }
    }
    (ro, watermarks)
}

/// The styles of `0x0206` (§16.14): the saved styles, the active style's
/// index, and the id → name map scenes refer to.
pub(crate) struct Styles26 {
    pub styles: Vec<Style>,
    pub active: Option<usize>,
    pub names: HashMap<u32, String>,
    /// The current settings' watermarks (`0x697b`, the `<name>_1` document).
    pub current_watermarks: Vec<Watermark>,
}

pub(crate) fn styles(top: &Node<'_>, zip: &Archive<'_>) -> Result<Styles26, Error> {
    let mut out = Vec::new();
    let mut by_id = HashMap::new();
    let mut active = None;
    let mut current_watermarks = Vec::new();
    let Some(sec) = top.node(0x0206)? else {
        return Ok(Styles26 {
            styles: out,
            active,
            names: by_id,
            current_watermarks,
        });
    };
    let mgr = sec.need(0x6978)?;
    let active_id = uint(mgr.get(0x697A));
    if let Some(cur) = mgr.node(0x697B)? {
        if let Some(rec) = cur.node(0x6B6C)? {
            current_watermarks = style_settings(zip, &text(rec.get(0x6B6F))).1;
        }
    }
    if let Some(list) = mgr.node(0x6979)? {
        for st in list.nodes(0x6B6C)? {
            let id = entity_id(&st)?;
            let name = text(st.get(0x6B6F));
            let (settings, watermarks) = style_settings(zip, &name);
            by_id.insert(id, name.clone());
            if Some(id) == active_id {
                active = Some(out.len());
            }
            out.push(Style {
                name,
                description: text(st.get(0x6B6E)),
                guid: st
                    .get(0x6B6D)
                    .map(|g| g.data.iter().map(|b| format!("{b:02x}")).collect())
                    .unwrap_or_default(),
                settings,
                watermarks,
            });
        }
    }
    Ok(Styles26 {
        styles: out,
        active,
        names: by_id,
        current_watermarks,
    })
}

pub(crate) fn axes(n: &Node<'_>) -> Result<Axes, Error> {
    Ok(Axes {
        origin_m: m3(f64s::<3>(n.get(0x4651), n.at)?),
        x: f64s::<3>(n.get(0x4652), n.at)?,
        y: f64s::<3>(n.get(0x4653), n.at)?,
        z: f64s::<3>(n.get(0x4654), n.at)?,
    })
}

/// The scenes of `0x0207` (§16.15).
pub(crate) fn scenes(
    top: &Node<'_>,
    style_names: &HashMap<u32, String>,
) -> Result<Vec<Scene>, Error> {
    let mut out = Vec::new();
    let Some(sec) = top.node(0x0207)? else {
        return Ok(out);
    };
    for pg in sec.need(0x6D60)?.need(0x6D61)?.nodes(0x7148)? {
        let base = pg.need(0x6F54)?;
        let saved = SceneProperties(uint(pg.get(0x7149)).unwrap_or(0));
        let camera = match pg.node(0x714A)? {
            Some(c) => Some(camera(&c.need(0x34BC)?)?),
            None => None,
        };
        let rendering = match pg.node(0x714D)? {
            Some(r) => Some(self::rendering(&r.need(0x733C)?)),
            None => None,
        };
        let shadows = match pg.node(0x714E)? {
            Some(s) => Some(shadow(&s.need(0x6590)?)?),
            None => None,
        };
        let axes = match pg.node(0x714F)? {
            Some(a) => Some(axes(&a.need(0x4650)?)?),
            None => None,
        };
        out.push(Scene {
            name: text(base.get(0x6F55)),
            description: text(base.get(0x6F56)),
            saved,
            camera,
            rendering,
            style: uint(pg.get(0x714C)).and_then(|id| style_names.get(&id).cloned()),
            shadows,
            axes,
            hidden_entities: ids(pg.get(0x714B)),
            active_section_planes: ids(pg.get(0x7151)),
            in_animation: flag(pg.get(0x7152)),
        });
    }
    Ok(out)
}

/// The fonts of `0x01fd` (§16.13) and their id → index map.
pub(crate) fn fonts(top: &Node<'_>) -> Result<(Vec<Font>, HashMap<u32, usize>), Error> {
    let mut out = Vec::new();
    let mut by_id = HashMap::new();
    let Some(sec) = top.node(0x01FD)? else {
        return Ok((out, by_id));
    };
    for f in sec.need(0x4E20)?.need(0x4E21)?.nodes(0x5014)? {
        by_id.insert(entity_id(&f)?, out.len());
        out.push(Font {
            family: text(f.get(0x5015)),
            bold: flag(f.get(0x5016)),
            italic: flag(f.get(0x5017)),
            size_pt: uint(f.get(0x5018)).unwrap_or(0),
        });
    }
    Ok((out, by_id))
}

fn anchor(n: Option<Node<'_>>, at: usize) -> Result<Anchor, Error> {
    let Some(a) = n else {
        return Ok(Anchor {
            kind: 0,
            point_m: [0.0; 3],
        });
    };
    let a = a.need(0x5208)?;
    let kind = uint(a.get(0x5209)).unwrap_or(0);
    let p = f64s::<3>(a.get(0x520A), at)?;
    Ok(Anchor {
        kind,
        point_m: if kind == 5 {
            [p[0], p[1] / INCH, p[2] / INCH]
        } else {
            m3(p)
        },
    })
}

/// The texts of an entity container (§16.13).
pub(crate) fn texts(container: &Node<'_>, fonts: &HashMap<u32, usize>) -> Result<Vec<Text>, Error> {
    let mut out = Vec::new();
    let Some(list) = container.node(0x1398)? else {
        return Ok(out);
    };
    for t in list.nodes(0x55F0)? {
        let leader = match uint(t.get(0x55FD)).unwrap_or(0) {
            1 => Leader::ViewBased,
            2 => Leader::Pushpin,
            _ => Leader::None,
        };
        let off = f64s::<3>(t.get(0x55F5), t.at)?;
        out.push(Text {
            pid: entity_id(&t.need(0x07D0)?)?,
            content: text(t.get(0x55F1)),
            screen_position: [
                f64_at(t.get(0x55F2)).unwrap_or(0.0),
                f64_at(t.get(0x55F3)).unwrap_or(0.0),
            ],
            anchor: anchor(t.node(0x55F4)?, t.at)?,
            leader_offset: if leader == Leader::Pushpin {
                m3(off)
            } else {
                off
            },
            leader,
            arrow: uint(t.get(0x55FA)).unwrap_or(0),
            font: uint(t.get(0x55F9)).and_then(|id| fonts.get(&id).copied()),
        });
    }
    Ok(out)
}

/// The linear dimensions of an entity container (§16.13).
pub(crate) fn dimensions(
    container: &Node<'_>,
    fonts: &HashMap<u32, usize>,
) -> Result<Vec<Dimension>, Error> {
    let mut out = Vec::new();
    let Some(list) = container.node(0x1399)? else {
        return Ok(out);
    };
    for d in list.nodes(0x5BCC)? {
        let base = d.need(0x59D8)?;
        out.push(Dimension {
            pid: entity_id(&base.need(0x07D0)?)?,
            text_override: text(base.get(0x59D9)),
            start: anchor(d.node(0x5BCD)?, d.at)?,
            end: anchor(d.node(0x5BCE)?, d.at)?,
            normal: f64s::<3>(d.get(0x5BCF), d.at)?,
            x_axis: f64s::<3>(d.get(0x5BD0), d.at)?,
            offset_m: f64_at(d.get(0x5BD2)).unwrap_or(0.0) / INCH,
            text_position: uint(d.get(0x5BD4)).unwrap_or(0),
            aligned: flag(base.get(0x59DB)),
            font: uint(base.get(0x59DA)).and_then(|id| fonts.get(&id).copied()),
        });
    }
    Ok(out)
}

/// A definition's `0x1b58` behaviour (§16.5).
pub(crate) fn behaviour(def: &Node<'_>) -> Result<Behaviour, Error> {
    let Some(b) = def.node(0x1B58)? else {
        return Ok(Behaviour::default());
    };
    Ok(Behaviour {
        glues_to_surface: flag(b.get(0x1B5B)),
        glue_plane: uint(b.get(0x1B59)).unwrap_or(0),
        cuts_opening: flag(b.get(0x1B5C)),
        always_faces_camera: flag(b.get(0x1B5D)),
        shadows_face_sun: flag(b.get(0x1B5E)),
    })
}

/// Defaults for new texts, `0x01fe > 0x57e4` (§16.13).
pub(crate) fn text_defaults(
    top: &Node<'_>,
    fonts: &HashMap<u32, usize>,
) -> Result<Option<TextDefaults>, Error> {
    let Some(sec) = top.node(0x01FE)? else {
        return Ok(None);
    };
    let Some(r) = sec.node(0x57E4)? else {
        return Ok(None);
    };
    let rgba = |t: u16| -> [u8; 4] {
        r.get(t)
            .and_then(|q| q.data.try_into().ok())
            .unwrap_or([0, 0, 0, 255])
    };
    Ok(Some(TextDefaults {
        font: uint(r.get(0x57E5)).and_then(|id| fonts.get(&id).copied()),
        screen_font: uint(r.get(0x57E6)).and_then(|id| fonts.get(&id).copied()),
        arrow: uint(r.get(0x57E7)).unwrap_or(0),
        leader_text_color: rgba(0x57EC),
        screen_text_color: rgba(0x57ED),
    }))
}

/// Defaults for new dimensions, `0x01ff > 0x5fb4` (§16.13).
pub(crate) fn dimension_defaults(
    top: &Node<'_>,
    fonts: &HashMap<u32, usize>,
) -> Result<Option<DimensionDefaults>, Error> {
    let Some(sec) = top.node(0x01FF)? else {
        return Ok(None);
    };
    let Some(r) = sec.node(0x5FB4)? else {
        return Ok(None);
    };
    Ok(Some(DimensionDefaults {
        font: uint(r.get(0x5FB5)).and_then(|id| fonts.get(&id).copied()),
        aligned: flag(r.get(0x5FB6)),
        arrow: uint(r.get(0x5FBB)).unwrap_or(0),
        text_position: uint(r.get(0x5FC6)).unwrap_or(0),
        color: r
            .get(0x5FC4)
            .and_then(|q| q.data.try_into().ok())
            .unwrap_or([0, 0, 0, 255]),
    }))
}
