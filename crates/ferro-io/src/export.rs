//! Mesh export. Both formats are written Y-up in metres (the glTF convention,
//! also what most OBJ importers expect): Ferro (x, y, z) mm → (x, z, −y) m.

use crate::IoError;
use ferro_core::algo::{body_mesh, Mesh};
use ferro_core::model::Document;
use ferro_core::Point3;
use serde_json::json;
use std::fmt::Write as _;

fn yup(p: Point3) -> [f32; 3] {
    [
        (p.x / 1000.0) as f32,
        (p.z / 1000.0) as f32,
        (-p.y / 1000.0) as f32,
    ]
}
fn yup_n(n: ferro_core::Vec3) -> [f32; 3] {
    [n.x as f32, n.z as f32, -n.y as f32]
}

fn meshes(doc: &Document) -> Result<Vec<(String, Mesh)>, IoError> {
    let mut out = Vec::new();
    for e in doc.elements() {
        // Voids (Forms that only cut, M5-5.3) are not exported.
        if ferro_core::model::cut::is_void(doc, e) {
            continue;
        }
        if let Some(g) = e.geometry() {
            let m = body_mesh(g).map_err(|err| IoError::new("ferro.io.mesh", err.to_string()))?;
            if !m.indices.is_empty() {
                out.push((e.label(), m));
            }
        }
    }
    Ok(out)
}

/// Wavefront OBJ text, one object per Form.
pub fn export_obj(doc: &Document) -> Result<String, IoError> {
    let mut s = String::from("# Exported by Ferro (Y-up, metres)\n");
    let mut base = 1usize;
    let mut nbase = 1usize;
    for (name, m) in meshes(doc)? {
        let clean: String = name
            .chars()
            .map(|c| if c.is_whitespace() { '_' } else { c })
            .collect();
        let _ = writeln!(s, "o {clean}");
        for p in &m.positions {
            let [x, y, z] = yup(*p);
            let _ = writeln!(s, "v {x} {y} {z}");
        }
        for n in &m.normals {
            let [x, y, z] = yup_n(*n);
            let _ = writeln!(s, "vn {x} {y} {z}");
        }
        for t in &m.indices {
            let [a, b, c] = t.map(|i| i as usize);
            let _ = writeln!(
                s,
                "f {}//{} {}//{} {}//{}",
                base + a,
                nbase + a,
                base + b,
                nbase + b,
                base + c,
                nbase + c
            );
        }
        base += m.positions.len();
        nbase += m.normals.len();
    }
    Ok(s)
}

/// Binary glTF 2.0 (GLB), one node + mesh per Form.
pub fn export_glb(doc: &Document) -> Result<Vec<u8>, IoError> {
    let mut bin: Vec<u8> = Vec::new();
    let mut buffer_views = Vec::new();
    let mut accessors = Vec::new();
    let mut gl_meshes = Vec::new();
    let mut nodes = Vec::new();
    let mut push_view = |bin: &mut Vec<u8>, data: &[u8], target: u32| -> usize {
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }
        let offset = bin.len();
        bin.extend_from_slice(data);
        buffer_views
            .push(json!({"buffer":0,"byteOffset":offset,"byteLength":data.len(),"target":target}));
        buffer_views.len() - 1
    };
    for (name, m) in meshes(doc)? {
        let pos: Vec<[f32; 3]> = m.positions.iter().map(|p| yup(*p)).collect();
        let nor: Vec<[f32; 3]> = m.normals.iter().map(|n| yup_n(*n)).collect();
        let idx: Vec<u32> = m.indices.iter().flatten().copied().collect();
        let mut min = [f32::MAX; 3];
        let mut max = [f32::MIN; 3];
        for p in &pos {
            for k in 0..3 {
                min[k] = min[k].min(p[k]);
                max[k] = max[k].max(p[k]);
            }
        }
        let bytes = |v: &[[f32; 3]]| {
            v.iter()
                .flatten()
                .flat_map(|f| f.to_le_bytes())
                .collect::<Vec<u8>>()
        };
        let pv = push_view(&mut bin, &bytes(&pos), 34962);
        let nv = push_view(&mut bin, &bytes(&nor), 34962);
        let iv = push_view(
            &mut bin,
            &idx.iter()
                .flat_map(|i| i.to_le_bytes())
                .collect::<Vec<u8>>(),
            34963,
        );
        let a0 = accessors.len();
        accessors.push(json!({"bufferView":pv,"componentType":5126,"count":pos.len(),"type":"VEC3","min":min,"max":max}));
        accessors
            .push(json!({"bufferView":nv,"componentType":5126,"count":nor.len(),"type":"VEC3"}));
        accessors
            .push(json!({"bufferView":iv,"componentType":5125,"count":idx.len(),"type":"SCALAR"}));
        gl_meshes.push(json!({"name":name,"primitives":[{"attributes":{"POSITION":a0,"NORMAL":a0+1},"indices":a0+2,"material":0}]}));
        nodes.push(json!({"name":name,"mesh":gl_meshes.len()-1}));
    }
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    let root = json!({
        "asset": {"version":"2.0","generator":"Ferro"},
        "scene": 0,
        "scenes": [{"name": doc.project().name, "nodes": (0..nodes.len()).collect::<Vec<_>>()}],
        "nodes": nodes,
        "meshes": gl_meshes,
        "materials": [{"name":"Default","pbrMetallicRoughness":{"baseColorFactor":[0.85,0.85,0.82,1.0],"metallicFactor":0.0,"roughnessFactor":0.8}}],
        "accessors": accessors,
        "bufferViews": buffer_views,
        "buffers": [{"byteLength": bin.len()}],
    });
    let mut js = serde_json::to_vec(&root)?;
    while !js.len().is_multiple_of(4) {
        js.push(b' ');
    }
    let total = 12 + 8 + js.len() + 8 + bin.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(js.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&js);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&bin);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::tests::sample;

    #[test]
    fn obj_has_box_geometry() {
        let s = sample();
        let obj = export_obj(s.doc()).unwrap();
        assert_eq!(obj.lines().filter(|l| l.starts_with("f ")).count(), 12);
        assert!(obj.contains("o Tầng_2"));
    }

    #[test]
    fn glb_header_and_json_are_valid() {
        let s = sample();
        let glb = export_glb(s.doc()).unwrap();
        assert_eq!(&glb[0..4], b"glTF");
        let total = u32::from_le_bytes(glb[8..12].try_into().unwrap()) as usize;
        assert_eq!(total, glb.len());
        let jlen = u32::from_le_bytes(glb[12..16].try_into().unwrap()) as usize;
        let v: serde_json::Value = serde_json::from_slice(&glb[20..20 + jlen]).unwrap();
        assert_eq!(v["meshes"].as_array().unwrap().len(), 1);
        assert_eq!(v["accessors"][2]["count"], 36);
    }
}
