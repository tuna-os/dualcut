//! `vellosrc`: a GStreamer push source that renders vector shapes with
//! Vello per frame (feature = "vector").
//!
//! Registered with a `vello://` URI handler so uridecodebin — and therefore
//! GES `UriClip`s — can instantiate it:
//!
//!   vello://star?fill=%23ffd700&w=220&h=220&spin=1
//!
//! Frames render at the buffer's timestamp, so the output is genuinely
//! per-frame (`spin=1` animates a rotation as a live-rendering proof).

use crate::document::ShapeKind;
use gst::glib;
use gst::prelude::*;
use gst::subclass::prelude::*;
use gst_base::prelude::*;
use gst_base::subclass::base_src::CreateSuccess;
use gst_base::subclass::prelude::*;
use gstreamer as gst;
use gstreamer_base as gst_base;
use std::sync::Mutex;

const DEFAULT_W: u32 = 200;
const DEFAULT_H: u32 = 200;
const FPS: u64 = 30;
/// Nominal stream length; clips use the window they need.
const DURATION_SECS: u64 = 3600;

#[derive(Clone)]
struct Settings {
    shape: ShapeKind,
    fill: String,
    width: u32,
    height: u32,
    spin: bool,
    uri: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            shape: ShapeKind::Rect,
            fill: "#5468ff".into(),
            width: DEFAULT_W,
            height: DEFAULT_H,
            spin: false,
            uri: None,
        }
    }
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct VelloSrc {
        pub(super) settings: Mutex<Settings>,
        pub(super) frame: Mutex<u64>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for VelloSrc {
        const NAME: &'static str = "DualcutVelloSrc";
        type Type = super::VelloSrc;
        type ParentType = gst_base::PushSrc;
        type Interfaces = (gst::URIHandler,);
    }

    impl ObjectImpl for VelloSrc {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPS: std::sync::OnceLock<Vec<glib::ParamSpec>> = std::sync::OnceLock::new();
            PROPS.get_or_init(|| {
                vec![
                    glib::ParamSpecString::builder("fill")
                        .nick("Fill color")
                        .blurb("Shape fill as #rrggbb / #aarrggbb; changeable while playing")
                        .build(),
                    glib::ParamSpecBoolean::builder("spin")
                        .nick("Spin")
                        .blurb("Rotate the shape continuously")
                        .build(),
                ]
            })
        }

        fn set_property(&self, _id: usize, value: &glib::Value, pspec: &glib::ParamSpec) {
            let mut s = self.settings.lock().unwrap();
            match pspec.name() {
                "fill" => s.fill = value.get::<String>().unwrap_or_default(),
                "spin" => s.spin = value.get::<bool>().unwrap_or_default(),
                _ => {}
            }
        }

        fn property(&self, _id: usize, pspec: &glib::ParamSpec) -> glib::Value {
            let s = self.settings.lock().unwrap();
            match pspec.name() {
                "fill" => s.fill.to_value(),
                "spin" => s.spin.to_value(),
                _ => unreachable!(),
            }
        }

        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();
            obj.set_live(false);
            obj.set_format(gst::Format::Time);
        }
    }

    impl GstObjectImpl for VelloSrc {}

    impl ElementImpl for VelloSrc {
        fn metadata() -> Option<&'static gst::subclass::ElementMetadata> {
            static METADATA: std::sync::OnceLock<gst::subclass::ElementMetadata> =
                std::sync::OnceLock::new();
            Some(METADATA.get_or_init(|| {
                gst::subclass::ElementMetadata::new(
                    "Vello vector source",
                    "Source/Video",
                    "Renders vector shapes with Vello per frame",
                    "dualcut",
                )
            }))
        }

        fn pad_templates() -> &'static [gst::PadTemplate] {
            static TEMPLATES: std::sync::OnceLock<Vec<gst::PadTemplate>> =
                std::sync::OnceLock::new();
            TEMPLATES.get_or_init(|| {
                let caps = gst::Caps::builder("video/x-raw")
                    .field("format", "RGBA")
                    .field("width", gst::IntRange::new(1, i32::MAX))
                    .field("height", gst::IntRange::new(1, i32::MAX))
                    .field("framerate", gst::Fraction::new(FPS as i32, 1))
                    .build();
                vec![gst::PadTemplate::new(
                    "src",
                    gst::PadDirection::Src,
                    gst::PadPresence::Always,
                    &caps,
                )
                .unwrap()]
            })
        }
    }

    impl BaseSrcImpl for VelloSrc {
        fn caps(&self, _filter: Option<&gst::Caps>) -> Option<gst::Caps> {
            let s = self.settings.lock().unwrap();
            Some(
                gst::Caps::builder("video/x-raw")
                    .field("format", "RGBA")
                    .field("width", s.width as i32)
                    .field("height", s.height as i32)
                    .field("framerate", gst::Fraction::new(FPS as i32, 1))
                    .build(),
            )
        }

        fn size(&self) -> Option<u64> {
            None
        }

        fn is_seekable(&self) -> bool {
            true
        }

        fn query(&self, query: &mut gst::QueryRef) -> bool {
            if let gst::QueryViewMut::Duration(q) = query.view_mut()
                && q.format() == gst::Format::Time {
                    q.set(gst::ClockTime::from_seconds(DURATION_SECS));
                    return true;
                }
            BaseSrcImplExt::parent_query(self, query)
        }

        fn do_seek(&self, segment: &mut gst::Segment) -> bool {
            let Some(segment) = segment.downcast_mut::<gst::format::Time>() else {
                return false;
            };
            let start = segment.start().unwrap_or(gst::ClockTime::ZERO);
            *self.frame.lock().unwrap() = start.nseconds() * FPS / 1_000_000_000;
            true
        }

        fn start(&self) -> Result<(), gst::ErrorMessage> {
            *self.frame.lock().unwrap() = 0;
            Ok(())
        }
    }

    impl PushSrcImpl for VelloSrc {
        fn create(
            &self,
            _buffer: Option<&mut gst::BufferRef>,
        ) -> Result<CreateSuccess, gst::FlowError> {
            let settings = self.settings.lock().unwrap().clone();
            let frame = {
                let mut f = self.frame.lock().unwrap();
                let cur = *f;
                *f += 1;
                cur
            };
            if frame >= DURATION_SECS * FPS {
                return Err(gst::FlowError::Eos);
            }
            let t = frame as f64 / FPS as f64;
            let rotate = if settings.spin { t * std::f64::consts::PI } else { 0.0 };

            let pixels = super::render_frame(&settings, rotate).map_err(|e| {
                gst::error!(gst::CAT_DEFAULT, "vello render failed: {e:#}");
                gst::FlowError::Error
            })?;

            let mut buffer = gst::Buffer::from_mut_slice(pixels);
            {
                let buffer = buffer.get_mut().unwrap();
                let pts = gst::ClockTime::from_nseconds(frame * 1_000_000_000 / FPS);
                buffer.set_pts(pts);
                buffer.set_duration(gst::ClockTime::from_nseconds(1_000_000_000 / FPS));
            }
            Ok(CreateSuccess::NewBuffer(buffer))
        }
    }

    impl URIHandlerImpl for VelloSrc {
        const URI_TYPE: gst::URIType = gst::URIType::Src;

        fn protocols() -> &'static [&'static str] {
            &["vello"]
        }

        fn uri(&self) -> Option<String> {
            self.settings.lock().unwrap().uri.clone()
        }

        fn set_uri(&self, uri: &str) -> Result<(), glib::Error> {
            let parsed = url::Url::parse(uri).map_err(|e| {
                glib::Error::new(gst::URIError::BadUri, &format!("invalid vello uri: {e}"))
            })?;
            let shape = match parsed.host_str().unwrap_or("rect") {
                "rect" => ShapeKind::Rect,
                "circle" => ShapeKind::Circle,
                "ellipse" => ShapeKind::Ellipse,
                "star" => ShapeKind::Star,
                "polygon" => ShapeKind::Polygon,
                "line" => ShapeKind::Line,
                "arrow" => ShapeKind::Arrow,
                other => {
                    return Err(glib::Error::new(
                        gst::URIError::BadUri,
                        &format!("unknown shape {other:?}"),
                    ))
                }
            };
            let mut s = self.settings.lock().unwrap();
            s.shape = shape;
            for (k, v) in parsed.query_pairs() {
                match k.as_ref() {
                    "fill" => s.fill = v.into_owned(),
                    "w" => s.width = v.parse().unwrap_or(DEFAULT_W),
                    "h" => s.height = v.parse().unwrap_or(DEFAULT_H),
                    "spin" => s.spin = v == "1" || v == "true",
                    _ => {}
                }
            }
            s.uri = Some(uri.to_string());
            Ok(())
        }
    }
}

glib::wrapper! {
    pub struct VelloSrc(ObjectSubclass<imp::VelloSrc>)
        @extends gst_base::PushSrc, gst_base::BaseSrc, gst::Element, gst::Object,
        @implements gst::URIHandler;
}

/// Register the element + URI handler; call once after gst::init.
pub fn register() -> Result<(), glib::BoolError> {
    gst::Element::register(
        None,
        "vellosrc",
        gst::Rank::NONE,
        VelloSrc::static_type(),
    )
}

fn render_frame(settings: &Settings, rotate: f64) -> anyhow::Result<Vec<u8>> {
    crate::vector::render_shape_rgba(
        settings.shape,
        &settings.fill,
        settings.width,
        settings.height,
        rotate,
    )
}
