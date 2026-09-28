//! `openskp` — command-line reader for SketchUp `.skp` files.
//!
//!   openskp id    <file>   header (version + model GUID) and class count
//!   openskp model <file>   human-readable decode summary
//!   openskp json  <file>   full model as JSON (stdout)
//!   openskp mesh  <file>   concrete mesh + composed scene as JSON (stdout)

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: openskp <id|model|json|mesh> <file.skp>");
        return ExitCode::FAILURE;
    }
    let (cmd, path) = (args[1].as_str(), args[2].as_str());
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("openskp: cannot read {path}: {e}");
            return ExitCode::FAILURE;
        }
    };

    match cmd {
        "id" => id(&data),
        "json" => match openskp::Model::parse(&data) {
            Ok(m) => {
                println!("{}", m.to_json());
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("openskp: {e}");
                ExitCode::FAILURE
            }
        },
        "mesh" => match openskp::Model::parse(&data) {
            Ok(m) => {
                println!("{}", m.mesh_json());
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("openskp: {e}");
                ExitCode::FAILURE
            }
        },
        "model" => match openskp::Model::parse(&data) {
            Ok(m) => {
                summary(path, &m, &data);
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("openskp: {e}");
                ExitCode::FAILURE
            }
        },
        other => {
            eprintln!("openskp: unknown command {other:?} (id|model|json|mesh)");
            ExitCode::FAILURE
        }
    }
}

fn id(data: &[u8]) -> ExitCode {
    // Header-only identification first: `id` must say WHAT a file is even
    // when the container is one this crate cannot read.
    let Some((version, guid)) = openskp::header_info(data) else {
        eprintln!("openskp: malformed .skp header");
        return ExitCode::FAILURE;
    };
    println!("version   {version}");
    if let Some(guid) = guid {
        println!("model     {guid}");
    }
    match openskp::detect_container(data) {
        openskp::Container::Carchive2017 => {
            let classes = openskp::inventory(data).map(|v| v.len()).unwrap_or(0);
            println!("classes   {classes}");
        }
        openskp::Container::Zip => {
            println!("container SketchUp 2026 (ZIP archive, model.dat record tree)");
        }
        openskp::Container::Unknown => {
            println!("container unknown — id only; not readable here");
        }
    }
    ExitCode::SUCCESS
}

fn summary(path: &str, m: &openskp::Model, data: &[u8]) {
    let name = std::path::Path::new(path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string());
    println!("== {name} ==  version {}", m.version);
    if openskp::detect_container(data) == openskp::Container::Zip {
        // Geometry offsets below are positions in the archive's model.dat.
        println!("container: SketchUp 2026 (ZIP archive, model.dat record tree)");
    } else {
        println!(
            "inventory: {} classes",
            openskp::inventory(data).map(|v| v.len()).unwrap_or(0)
        );
    }
    // Which walk served the file (SKP_FORMAT §4s) — never silent.
    for d in &m.diagnostics {
        match d {
            openskp::Diagnostic::ContinuousWalk { base } => {
                println!("walk: continuous single-archive (base {base} pre-model slots)");
            }
            openskp::Diagnostic::ContinuousFallback { stage, at, detail } => {
                println!(
                    "walk: legacy runs (continuous attempt died in {stage} @0x{at:x}: {detail})"
                );
            }
            openskp::Diagnostic::PadSlotBound { slot, class } => {
                println!("  pad slot {slot} bound -> {class}");
            }
            openskp::Diagnostic::MaterialLinkFallback {
                declared,
                extracted,
            } => {
                println!("  material slot arithmetic fell back to suffix alignment (declared {declared}, extracted {extracted})");
            }
            _ => {}
        }
    }
    // Phase 0.4: anomalies are never silent. Desync diagnostics are loud;
    // informational run-filtering is summarized in one line.
    let desync: Vec<_> = m.diagnostics.iter().filter(|d| d.is_desync()).collect();
    if !desync.is_empty() {
        println!("DIAGNOSTICS: {} desync event(s):", desync.len());
        for d in desync.iter().take(8) {
            println!("  {d:?}");
        }
        if desync.len() > 8 {
            println!("  … {} more", desync.len() - 8);
        }
    }
    let filtered = m
        .diagnostics
        .iter()
        .filter(|d| matches!(d, openskp::Diagnostic::RunFiltered { .. }))
        .count();
    if filtered > 0 {
        println!("note: {filtered} candidate run(s) filtered (degenerate/low-confidence)");
    }
    if !m.definitions.is_empty() {
        let names: Vec<_> = m
            .definitions
            .iter()
            .map(|d| format!("{:?}", d.name))
            .collect();
        println!("definitions: {}", names.join(", "));
    }
    for i in &m.instances {
        let kind = match i.is_group {
            Some(true) => "group",
            Some(false) => "component",
            None => "instance",
        };
        match i.name.as_deref() {
            Some(n) if !n.is_empty() => println!(
                "  {kind}: {n:?} places {:?} @ T={:?} m",
                i.definition, i.translation_m
            ),
            _ => println!("  {kind}: {:?} @ T={:?} m", i.definition, i.translation_m),
        }
    }
    for r in &m.geometry {
        let t = &r.topology;
        println!(
            "  geometry @0x{:x}: {}V {}E {}F {}L {}EU {}C  (back-refs {}/{})",
            r.start,
            t.vertices,
            t.edges,
            t.faces,
            t.loops,
            t.edge_uses,
            t.curves,
            r.resolved.0,
            r.resolved.1
        );
    }
    if !m.materials.is_empty() {
        println!("materials: {}", m.materials.len());
        for mat in &m.materials {
            match mat {
                openskp::Material::Solid {
                    name,
                    rgba,
                    opacity,
                } => {
                    println!("  {name:?} solid rgba={rgba:?} opacity={opacity}");
                }
                openskp::Material::Textured {
                    name,
                    texture,
                    image_bytes: _,
                    avg_rgba: _,
                    applied_size_in,
                    opacity,
                } => {
                    println!(
                        "  {name:?} textured texture={texture:?} size={applied_size_in:?} \
                         opacity={opacity}"
                    );
                }
            }
        }
    }
    if !m.layers.is_empty() {
        let names: Vec<_> = m.layers.iter().map(|l| l.name.clone()).collect();
        println!("layers: {names:?}");
    }
    if let Some(c) = &m.camera {
        println!(
            "camera: eye {:?} target {:?} {} fov {:.1}",
            round3(c.eye_m),
            round3(c.target_m),
            if c.perspective {
                "perspective"
            } else {
                "parallel"
            },
            c.fov_deg
        );
    }
    if let Some(r) = &m.rendering {
        println!(
            "rendering: face style {} edges {} profiles {} sky {} ground {} shadows {}",
            r.face_style,
            r.edges,
            r.profiles,
            r.sky,
            r.ground,
            m.shadows.as_ref().is_some_and(|s| s.displayed)
        );
    }
    if let Some(s) = &m.shadows {
        println!(
            "shadows: {} {} ({:.4}, {:.4}) UTC{:+} light {} dark {}",
            s.city, s.country, s.latitude, s.longitude, s.tz_offset_h, s.light, s.dark
        );
    }
    println!(
        "units: format {} unit {} precision {} snap {}",
        m.units.length_format, m.units.length_unit, m.units.length_precision, m.units.length_snap
    );
    if !m.styles.is_empty() {
        for (i, st) in m.styles.iter().enumerate() {
            println!(
                "style {i}{}: {:?} watermarks {}",
                if Some(i) == m.active_style {
                    " (active)"
                } else {
                    ""
                },
                st.name,
                st.watermarks.len()
            );
        }
    }
    for sc in &m.scenes {
        println!(
            "scene: {:?} saved {:#x}{}{}{}{}",
            sc.name,
            sc.saved.0,
            if sc.camera.is_some() { " camera" } else { "" },
            if sc.rendering.is_some() { " style" } else { "" },
            if sc.shadows.is_some() { " shadows" } else { "" },
            if sc.axes.is_some() { " axes" } else { "" },
        );
    }
    if !m.fonts.is_empty() || !m.texts.is_empty() || !m.dimensions.is_empty() {
        println!(
            "annotations: {} fonts, {} texts, {} dimensions",
            m.fonts.len(),
            m.texts.len(),
            m.dimensions.len()
        );
    }
    if !m.guides.is_empty() {
        println!("guides: {}", m.guides.len());
    }
    if !m.attributes.is_empty() {
        println!("attributes: {} key/values", m.attributes.len());
    }
    let kinds: Vec<_> = m.images.iter().map(|i| i.kind.clone()).collect();
    println!("images: {} embedded ({})", m.images.len(), kinds.join(", "));
}

fn round3(v: [f64; 3]) -> [f64; 3] {
    v.map(|x| (x * 1e3).round() / 1e3)
}
