//! Typed document settings, shared by the 2017 and post-2017 readers:
//! camera, rendering options, shadows, units, styles, watermarks and
//! scenes (SKP_FORMAT §10.6–§10.10 for the 2017 records, §16.9–§16.15
//! for the post-2017 ones). Both readers fill the same structs, and
//! `tests/read26.rs` holds every converted corpus file to its original.

use crate::INCH;

/// A camera (§10.6, §16.9). Positions are in metres, as mesh vertices are.
#[derive(Debug, Clone, PartialEq)]
pub struct Camera {
    pub eye_m: [f64; 3],
    pub target_m: [f64; 3],
    /// Unit up vector.
    pub up: [f64; 3],
    pub perspective: bool,
    /// Vertical field of view in degrees (perspective cameras).
    pub fov_deg: f64,
    /// View height in metres (parallel-projection cameras).
    pub parallel_height_m: f64,
    pub two_point_perspective: bool,
    pub description: String,
}

impl Camera {
    /// Decode the 2017 `CCamera` body at `body` (137 bytes) and its 33-byte
    /// tail at `tail`; `description` is the string between them.
    pub(crate) fn from_2017(
        d: &[u8],
        body: usize,
        tail: usize,
        description: String,
    ) -> Option<Camera> {
        let f = |o: usize| -> Option<f64> {
            Some(f64::from_le_bytes(d.get(o..o + 8)?.try_into().ok()?))
        };
        let v3 = |o: usize| -> Option<[f64; 3]> { Some([f(o)?, f(o + 8)?, f(o + 16)?]) };
        let m = |v: [f64; 3]| [v[0] / INCH, v[1] / INCH, v[2] / INCH];
        Some(Camera {
            eye_m: m(v3(body)?),
            target_m: m(v3(body + 24)?),
            up: v3(body + 48)?,
            perspective: *d.get(body + 88)? != 0,
            fov_deg: f(body + 89)?,
            parallel_height_m: f(body + 97)? / INCH,
            two_point_perspective: *d.get(tail + 8)? != 0,
            description,
        })
    }
}

/// One rendering-option field: its post-2017 record tag, its 2017 offset in
/// the §10.7 block (`None` for fields the 2017 block lacks) and the style
/// document item id that carries it (`None` for fields no style holds).
#[derive(Debug, Clone, Copy)]
pub(crate) struct RoField {
    pub name: &'static str,
    pub kind: RoKind,
    pub tag: u16,
    pub off17: Option<usize>,
    pub item: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RoKind {
    Bool,
    U32,
    Rgba,
    F64,
}

/// A decoded value for one rendering-option field.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum RoValue {
    Bool(bool),
    U32(u32),
    Rgba([u8; 4]),
    F64(f64),
}

impl RoValue {
    /// Decode raw little-endian bytes of the field's kind; `None` when the
    /// width does not match.
    pub(crate) fn decode(kind: RoKind, b: &[u8]) -> Option<RoValue> {
        Some(match kind {
            RoKind::Bool => RoValue::Bool(*b.first()? != 0),
            RoKind::U32 => RoValue::U32(u32::from_le_bytes(b.get(..4)?.try_into().ok()?)),
            RoKind::Rgba => RoValue::Rgba(b.get(..4)?.try_into().ok()?),
            RoKind::F64 => RoValue::F64(f64::from_le_bytes(b.get(..8)?.try_into().ok()?)),
        })
    }
    /// Decode a style document item's text (`<t:variant>`): integers for
    /// bools, u32s and colours (signed, as the XML writes them), decimals
    /// for f64s.
    pub(crate) fn parse_item(kind: RoKind, text: &str) -> Option<RoValue> {
        let t = text.trim();
        Some(match kind {
            RoKind::Bool => RoValue::Bool(t.parse::<i64>().ok()? != 0),
            RoKind::U32 => RoValue::U32(t.parse::<i64>().ok()? as u32),
            RoKind::Rgba => RoValue::Rgba((t.parse::<i64>().ok()? as u32).to_le_bytes()),
            RoKind::F64 => RoValue::F64(t.parse::<f64>().ok()?),
        })
    }
}

macro_rules! rendering_options {
    ($( $(#[$doc:meta])* $name:ident : $ty:ty = $kind:ident, $tag:literal, $off:expr, $item:expr; )*) => {
        /// The display settings of the document or of a style (§10.7, §16.10).
        /// Every field is pinned by a one-change pair; the fields marked
        /// undecoded in the spec are not exposed.
        #[derive(Debug, Clone, PartialEq, Default)]
        pub struct RenderingOptions {
            $( $(#[$doc])* pub $name: $ty, )*
        }

        pub(crate) const RO_FIELDS: &[RoField] = &[
            $( RoField { name: stringify!($name), kind: RoKind::$kind, tag: $tag, off17: $off, item: $item }, )*
        ];

        impl RenderingOptions {
            /// Store one decoded field by name.
            pub(crate) fn set(&mut self, name: &str, v: RoValue) {
                match name {
                    $( stringify!($name) => { if let Some(x) = <$ty as RoStore>::from_value(v) { self.$name = x; } } )*
                    _ => {}
                }
            }
            /// Every field as `(name, value)`, in table order.
            pub(crate) fn values(&self) -> Vec<(&'static str, Option<RoValue>)> {
                vec![ $( (stringify!($name), <$ty as RoStore>::to_value(&self.$name)), )* ]
            }
        }
    };
}

/// Conversion between a field's Rust type and [`RoValue`].
pub(crate) trait RoStore: Sized {
    fn from_value(v: RoValue) -> Option<Self>;
    fn to_value(&self) -> Option<RoValue>;
}
impl RoStore for bool {
    fn from_value(v: RoValue) -> Option<Self> {
        match v {
            RoValue::Bool(b) => Some(b),
            RoValue::U32(x) => Some(x != 0),
            _ => None,
        }
    }
    fn to_value(&self) -> Option<RoValue> {
        Some(RoValue::Bool(*self))
    }
}
impl RoStore for u32 {
    fn from_value(v: RoValue) -> Option<Self> {
        match v {
            RoValue::U32(x) => Some(x),
            RoValue::Bool(b) => Some(b as u32),
            _ => None,
        }
    }
    fn to_value(&self) -> Option<RoValue> {
        Some(RoValue::U32(*self))
    }
}
impl RoStore for [u8; 4] {
    fn from_value(v: RoValue) -> Option<Self> {
        match v {
            RoValue::Rgba(c) => Some(c),
            _ => None,
        }
    }
    fn to_value(&self) -> Option<RoValue> {
        Some(RoValue::Rgba(*self))
    }
}
impl RoStore for f64 {
    fn from_value(v: RoValue) -> Option<Self> {
        match v {
            RoValue::F64(x) => Some(x),
            _ => None,
        }
    }
    fn to_value(&self) -> Option<RoValue> {
        Some(RoValue::F64(*self))
    }
}
impl<T: RoStore> RoStore for Option<T> {
    fn from_value(v: RoValue) -> Option<Self> {
        Some(T::from_value(v))
    }
    fn to_value(&self) -> Option<RoValue> {
        self.as_ref().and_then(T::to_value)
    }
}

rendering_options! {
    /// 0 wireframe, 1 hidden line, 2 shaded, 5 monochrome; textures are
    /// `textures` below.
    face_style: u32 = U32, 0x733d, Some(0), Some(2001);
    xray: bool = Bool, 0x733e, Some(4), Some(2004);
    material_transparency: bool = Bool, 0x733f, Some(5), Some(2005);
    /// Shaded with textures (with `face_style` 2).
    textures: bool = Bool, 0x7340, Some(30), Some(2007);
    edges: bool = U32, 0x7341, Some(7), Some(1000);
    back_edges: bool = Bool, 0x7356, Some(187), Some(1015);
    /// 0 by material, 1 all the same colour, 2 by axis.
    edge_color_mode: u32 = U32, 0x7348, Some(31), Some(1002);
    edge_color: [u8; 4] = Rgba, 0x7358, Some(15), Some(1014);
    profiles: bool = Bool, 0x734f, Some(40), Some(1006);
    profiles_width: u32 = U32, 0x7350, Some(41), Some(1007);
    depth_cue: bool = Bool, 0x7351, Some(45), Some(1008);
    depth_cue_width: u32 = U32, 0x7352, Some(46), Some(1009);
    extension: bool = Bool, 0x734d, Some(35), Some(1004);
    extension_length: u32 = U32, 0x734e, Some(36), Some(1005);
    endpoints: bool = Bool, 0x7353, Some(50), Some(1010);
    endpoints_length: u32 = U32, 0x7354, Some(51), Some(1011);
    jitter: bool = Bool, 0x734b, Some(6), Some(1012);
    front_color: [u8; 4] = Rgba, 0x735c, Some(61), Some(2002);
    back_color: [u8; 4] = Rgba, 0x735d, Some(65), Some(2003);
    /// 0 faster, 2 nicer.
    transparency_quality: u32 = U32, 0x7377, Some(160), Some(2006);
    /// Candidate: X-ray opacity (the control cannot be changed in
    /// SketchUp Make 2017).
    xray_opacity: f64 = F64, 0x7378, Some(179), Some(2008);
    background: [u8; 4] = Rgba, 0x7357, Some(11), Some(4000);
    sky: bool = Bool, 0x7368, Some(133), Some(4002);
    sky_color: [u8; 4] = Rgba, 0x7365, Some(121), Some(4001);
    ground: bool = Bool, 0x7369, Some(134), Some(4004);
    ground_color: [u8; 4] = Rgba, 0x7367, Some(129), Some(4003);
    /// 0–100.
    ground_transparency: u32 = U32, 0x736b, Some(136), Some(4005);
    ground_from_below: bool = Bool, 0x736a, Some(135), Some(4006);
    watermarks: bool = Bool, 0x735e, Some(178), Some(5000);
    selected_color: [u8; 4] = Rgba, 0x7359, Some(19), Some(7000);
    locked_color: [u8; 4] = Rgba, 0x735a, Some(174), Some(7001);
    guides_color: [u8; 4] = Rgba, 0x735b, Some(23), Some(7002);
    guides_hidden: bool = Bool, 0x7346, Some(120), Some(7012);
    model_axes: bool = Bool, 0x7343, Some(117), Some(7008);
    color_by_layer: bool = Bool, 0x7347, Some(29), Some(7011);
    hidden_geometry: bool = Bool, 0x7380, Some(56), Some(7017);
    /// Post-2017 only; a 2017 file's single hidden-geometry setting fills
    /// both.
    hidden_objects: bool = Bool, 0x7381, Some(56), Some(7018);
    active_section_color: [u8; 4] = Rgba, 0x7370, Some(140), Some(7003);
    inactive_section_color: [u8; 4] = Rgba, 0x7371, Some(144), Some(7004);
    section_cut_color: [u8; 4] = Rgba, 0x7372, Some(148), Some(7005);
    section_cut_width: u32 = U32, 0x7374, Some(152), Some(7014);
    /// Bit 1 section planes shown, bit 2 section cuts shown.
    section_display: u32 = U32, 0x7375, Some(156), Some(7013);
    /// Post-2017 only.
    section_fill: Option<bool> = Bool, 0x7376, None, Some(7015);
    /// Post-2017 only.
    section_fill_color: Option<[u8; 4]> = Rgba, 0x7373, None, Some(7016);
    fog: bool = Bool, 0x735f, Some(87), None;
    fog_color: [u8; 4] = Rgba, 0x7360, Some(88), None;
    fog_uses_background: bool = Bool, 0x7361, Some(92), None;
    /// Metres; −1 when unset.
    fog_start_m: f64 = F64, 0x7362, Some(93), None;
    /// Metres; −1 when unset.
    fog_end_m: f64 = F64, 0x7363, Some(101), None;
    /// Component editing: hide the rest of the model.
    hide_rest_of_model: bool = Bool, 0x736e, Some(85), None;
    /// Component editing: hide similar components.
    hide_similar_components: bool = Bool, 0x736f, Some(86), None;
    /// Show component axes.
    component_axes: bool = Bool, 0x7349, Some(28), None;
    /// Component editing: opacity of the rest of the model, 0–1.
    fade_rest_of_model: f64 = F64, 0x736c, Some(69), None;
    /// Component editing: opacity of similar components, 0–1.
    fade_similar_components: f64 = F64, 0x736d, Some(77), None;
    /// Match Photo.
    background_photo: bool = Bool, 0x737c, Some(188), Some(8000);
    background_photo_opacity: f64 = F64, 0x737d, Some(189), Some(8001);
    foreground_photo: bool = Bool, 0x737e, Some(197), Some(8002);
    foreground_photo_opacity: f64 = F64, 0x737f, Some(198), Some(8003);
}

impl RenderingOptions {
    /// The fields a style document carries, with every other field at its
    /// default: what a saved style's settings can say.
    pub fn style_view(&self) -> RenderingOptions {
        let mut out = RenderingOptions::default();
        for (f, (_, v)) in RO_FIELDS.iter().zip(self.values()) {
            if let (Some(_), Some(v)) = (f.item, v) {
                out.set(f.name, v);
            }
        }
        out
    }

    pub fn section_planes(&self) -> bool {
        self.section_display & 1 != 0
    }
    pub fn section_cuts(&self) -> bool {
        self.section_display & 2 != 0
    }

    /// Decode the 2017 block whose face-style u32 is at `base` (§10.7).
    pub(crate) fn from_2017(d: &[u8], base: usize) -> Option<RenderingOptions> {
        let mut ro = RenderingOptions::default();
        for f in RO_FIELDS {
            let Some(off) = f.off17 else { continue };
            let v = RoValue::decode(f.kind, d.get(base + off..)?)?;
            ro.set(f.name, v);
        }
        // fog distances are stored in inches
        ro.fog_start_m = inches_to_m_or_unset(ro.fog_start_m);
        ro.fog_end_m = inches_to_m_or_unset(ro.fog_end_m);
        Some(ro)
    }

    /// Store one post-2017 `0x733c` child record.
    pub(crate) fn set_record(&mut self, tag: u16, b: &[u8]) {
        if let Some(f) = RO_FIELDS.iter().find(|f| f.tag == tag) {
            if let Some(v) = RoValue::decode(f.kind, b) {
                let v = match (tag, v) {
                    (0x7362 | 0x7363, RoValue::F64(x)) => RoValue::F64(inches_to_m_or_unset(x)),
                    _ => v,
                };
                self.set(f.name, v);
            }
        }
    }

    /// Store one style item (`<sty:item id>` or a 2017 CSkpStyle item).
    pub(crate) fn set_item(&mut self, item: u32, v: RoValue) {
        for f in RO_FIELDS.iter().filter(|f| f.item == Some(item)) {
            self.set(f.name, v);
        }
    }
}

fn inches_to_m_or_unset(x: f64) -> f64 {
    if x < 0.0 {
        x
    } else {
        x / INCH
    }
}

/// The sun and shadow settings of the document or a scene (§10.9, §16.11).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ShadowInfo {
    /// Date and time, UNIX seconds.
    pub time: u32,
    pub city: String,
    pub country: String,
    pub longitude: f64,
    pub latitude: f64,
    /// Hours east of UTC.
    pub tz_offset_h: f64,
    pub displayed: bool,
    pub on_faces: bool,
    pub on_ground: bool,
    pub from_edges: bool,
    /// 0–100.
    pub light: u32,
    /// 0–100.
    pub dark: u32,
    pub use_sun_for_shading: bool,
}

/// The document's unit settings (`UnitsOptions`, §16.12).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Units {
    /// 0 decimal, 1 architectural, 2 engineering, 3 fractional.
    pub length_format: u32,
    /// 0 inches, 1 feet, 2 millimetres, 3 centimetres, 4 metres.
    pub length_unit: u32,
    /// Decimal places, or the power of two for fractions.
    pub length_precision: u32,
    pub angle_precision: u32,
    pub length_snap: bool,
    /// In the display unit.
    pub length_snap_length: f64,
    pub angle_snap: bool,
    /// Degrees.
    pub snap_angle: f64,
    pub suppress_units_display: bool,
    pub force_inch_display: bool,
    /// Post-2017 only: 0 in², 1 ft², 2 mm², 3 cm², 4 m², 5 yd².
    pub area_unit: Option<u32>,
    /// Post-2017 only: 0 in³ … 5 yd³, 6 litres, 7 US gallons.
    pub volume_unit: Option<u32>,
    pub area_precision: Option<u32>,
    pub volume_precision: Option<u32>,
}

impl Units {
    /// Build from the flat attribute list both readers produce.
    pub(crate) fn from_attributes(attrs: &[crate::Attribute]) -> Units {
        use crate::AttrValue::*;
        let mut u = Units::default();
        let int = |v: &crate::AttrValue| match v {
            Int(x) => Some(*x as u32),
            Bool(b) => Some(*b as u32),
            F64(_) => None,
        };
        let flt = |v: &crate::AttrValue| match v {
            F64(x) => Some(*x),
            Int(x) => Some(*x as f64),
            Bool(_) => None,
        };
        let boo = |v: &crate::AttrValue| match v {
            Bool(b) => Some(*b),
            Int(x) => Some(*x != 0),
            F64(_) => None,
        };
        for a in attrs {
            match a.key.as_str() {
                "LengthFormat" => u.length_format = int(&a.value).unwrap_or(0),
                "LengthUnit" => u.length_unit = int(&a.value).unwrap_or(0),
                "LengthPrecision" => u.length_precision = int(&a.value).unwrap_or(0),
                "AnglePrecision" => u.angle_precision = int(&a.value).unwrap_or(0),
                "LengthSnapEnabled" => u.length_snap = boo(&a.value).unwrap_or(false),
                "LengthSnapLength" => u.length_snap_length = flt(&a.value).unwrap_or(0.0),
                "AngleSnapEnabled" => u.angle_snap = boo(&a.value).unwrap_or(false),
                "SnapAngle" => u.snap_angle = flt(&a.value).unwrap_or(0.0),
                "SuppressUnitsDisplay" => u.suppress_units_display = boo(&a.value).unwrap_or(false),
                "ForceInchDisplay" => u.force_inch_display = boo(&a.value).unwrap_or(false),
                "AreaUnit" => u.area_unit = int(&a.value),
                "VolumeUnit" => u.volume_unit = int(&a.value),
                "AreaPrecision" => u.area_precision = int(&a.value),
                "VolumePrecision" => u.volume_precision = int(&a.value),
                _ => {}
            }
        }
        u
    }
}

/// A watermark of a style (§10.8, §16.14).
#[derive(Debug, Clone, PartialEq)]
pub struct Watermark {
    pub name: String,
    /// The image file the watermark was loaded from.
    pub source_file: String,
    /// True for a background watermark, false for an overlay.
    pub background: bool,
    pub tiled: bool,
    pub stretched: bool,
    pub keep_aspect_ratio: bool,
    /// Grid cell 0–8, row-major from the top left (4 = centre), for
    /// positioned watermarks.
    pub position: u32,
    /// Use the image brightness as a mask.
    pub mask: bool,
    /// Blend with the model, 0–1.
    pub blend: f64,
    /// Scale, 0–1.
    pub scale: f64,
}

/// A saved style (§10.8, §16.14): its display settings and watermarks.
#[derive(Debug, Clone, PartialEq)]
pub struct Style {
    pub name: String,
    pub description: String,
    /// 16 bytes, hex.
    pub guid: String,
    pub settings: RenderingOptions,
    pub watermarks: Vec<Watermark>,
}

/// The model axes: origin in metres and three unit directions.
#[derive(Debug, Clone, PartialEq)]
pub struct Axes {
    pub origin_m: [f64; 3],
    pub x: [f64; 3],
    pub y: [f64; 3],
    pub z: [f64; 3],
}

/// Which properties a scene saves (§16.15 `0x7149`); the bits a 2017 file
/// carries are 1–64.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SceneProperties(pub u32);

impl SceneProperties {
    pub const CAMERA: u32 = 1;
    pub const STYLE_AND_FOG: u32 = 2;
    pub const SHADOWS: u32 = 4;
    pub const AXES: u32 = 8;
    /// The 2017 hidden-geometry property.
    pub const HIDDEN_GEOMETRY: u32 = 16;
    pub const VISIBLE_LAYERS: u32 = 32;
    pub const ACTIVE_SECTION_PLANES: u32 = 64;
    /// Post-2017: hidden geometry and hidden objects as separate bits.
    pub const HIDDEN_GEOMETRY_2: u32 = 128;
    pub const HIDDEN_OBJECTS: u32 = 256;
    pub fn has(self, bit: u32) -> bool {
        self.0 & bit != 0
    }
}

/// A scene (§10.10, §16.15). Optional parts are present exactly when the
/// scene saves that property.
#[derive(Debug, Clone, PartialEq)]
pub struct Scene {
    pub name: String,
    pub description: String,
    pub saved: SceneProperties,
    pub camera: Option<Camera>,
    pub rendering: Option<RenderingOptions>,
    /// The name of the scene's style, when the style is saved.
    pub style: Option<String>,
    pub shadows: Option<ShadowInfo>,
    pub axes: Option<Axes>,
    /// Persistent ids of the entities the scene hides. Empty on the 2017
    /// legacy byte-scan path, which cannot resolve the references.
    pub hidden_entities: Vec<u32>,
    /// Persistent ids of the active section planes.
    pub active_section_planes: Vec<u32>,
    /// Indices into `Model::layers` of the layers the scene hides. Empty
    /// on the 2017 legacy byte-scan path.
    pub hidden_layers: Vec<usize>,
    pub in_animation: bool,
}

/// A text font (§10.1, §16.13).
#[derive(Debug, Clone, PartialEq)]
pub struct Font {
    pub family: String,
    pub bold: bool,
    pub italic: bool,
    /// Points.
    pub size_pt: u32,
}

/// What a text or dimension is attached to (§16.13 anchor).
#[derive(Debug, Clone, PartialEq)]
pub struct Anchor {
    /// 0 none, 2 a vertex, 5 a point on an edge.
    pub kind: u32,
    /// Metres; for kind 5 the first component is the edge parameter.
    pub point_m: [f64; 3],
    /// Persistent id of the anchored entity, when one is stored.
    pub entity: Option<u32>,
}

/// How a text is led to its anchor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Leader {
    None,
    ViewBased,
    Pushpin,
}

/// A text entity (§10.2, §16.13).
#[derive(Debug, Clone, PartialEq)]
pub struct Text {
    pub pid: u32,
    pub content: String,
    /// Screen position as fractions of the view (screen text).
    pub screen_position: [f64; 2],
    pub anchor: Anchor,
    /// Model-space offset for a pushpin leader, screen-space for a
    /// view-based one.
    pub leader_offset: [f64; 3],
    pub leader: Leader,
    /// 0 none, 1 slash, 2 dot, 3 closed, 4 open.
    pub arrow: u32,
    /// Index into `Model::fonts`.
    pub font: Option<usize>,
}

/// A linear dimension (§10.1, §16.13).
#[derive(Debug, Clone, PartialEq)]
pub struct Dimension {
    pub pid: u32,
    /// Empty for the automatic measurement text.
    pub text_override: String,
    pub start: Anchor,
    pub end: Anchor,
    pub normal: [f64; 3],
    pub x_axis: [f64; 3],
    /// Offset of the dimension line from the measured points, metres.
    pub offset_m: f64,
    /// 0 above, 1 centred, 3 below.
    pub text_position: u32,
    /// Text aligned to the dimension line (else to the screen).
    pub aligned: bool,
    /// Index into `Model::fonts`.
    pub font: Option<usize>,
}

/// A component definition's behaviour (§7.1, §16.5).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Behaviour {
    pub glues_to_surface: bool,
    /// 0 any, 1 horizontal, 2 vertical, 3 sloped.
    pub glue_plane: u32,
    pub cuts_opening: bool,
    pub always_faces_camera: bool,
    pub shadows_face_sun: bool,
}

/// Scene animation settings (`PageOptions` and `SlideshowOptions`, §16.12).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Animation {
    pub transitions: bool,
    /// Seconds.
    pub transition_s: f64,
    /// Seconds each scene is shown.
    pub delay_s: f64,
    pub loop_slideshow: bool,
}

impl Animation {
    pub(crate) fn from_attributes(attrs: &[crate::Attribute]) -> Animation {
        use crate::AttrValue::*;
        let mut a = Animation::default();
        for at in attrs {
            match (at.key.as_str(), &at.value) {
                ("ShowTransition", Bool(b)) => a.transitions = *b,
                ("ShowTransition", Int(x)) => a.transitions = *x != 0,
                ("TransitionTime", F64(x)) => a.transition_s = *x,
                ("SlideTime", F64(x)) => a.delay_s = *x,
                ("LoopSlideshow", Bool(b)) => a.loop_slideshow = *b,
                ("LoopSlideshow", Int(x)) => a.loop_slideshow = *x != 0,
                _ => {}
            }
        }
        a
    }
}

/// Defaults for new texts (§10.11, §16.13).
#[derive(Debug, Clone, PartialEq)]
pub struct TextDefaults {
    /// Index into `Model::fonts` (leader text).
    pub font: Option<usize>,
    /// Index into `Model::fonts` (screen text).
    pub screen_font: Option<usize>,
    /// 0 none, 1 slash, 2 dot, 3 closed, 4 open.
    pub arrow: u32,
    pub leader_text_color: [u8; 4],
    pub screen_text_color: [u8; 4],
}

/// Defaults for new dimensions (§10.11, §16.13).
#[derive(Debug, Clone, PartialEq)]
pub struct DimensionDefaults {
    /// Index into `Model::fonts`.
    pub font: Option<usize>,
    /// Text aligned to the dimension line (else to the screen).
    pub aligned: bool,
    /// 0 none, 1 slash, 2 dot, 3 closed, 4 open.
    pub arrow: u32,
    /// 0 above, 1 centred, 3 below.
    pub text_position: u32,
    pub color: [u8; 4],
}
