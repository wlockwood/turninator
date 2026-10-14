use crate::mesh::MAX_VERTICES;
use crate::{Result, mesh::Mesh};
use std::{collections::HashMap, fs, path::Path};

const MAX_STL_BYTES: u64 = 128 * 1024 * 1024;

#[derive(Debug)]
pub struct StlImport {
    pub mesh: Mesh,
    pub omitted_degenerate_triangles: usize,
}

pub fn load_stl_with_report(path: &Path, millimeters_per_unit: f64) -> Result<StlImport> {
    if !millimeters_per_unit.is_finite() || millimeters_per_unit <= 0. {
        return Err("STL unit scale must be positive and finite".into());
    }
    if fs::metadata(path)?.len() > MAX_STL_BYTES {
        return Err("STL exceeds 128 MiB input budget".into());
    }
    let bytes = fs::read(path)?;
    let is_binary = if bytes.len() >= 84 {
        let n = u32::from_le_bytes(bytes[80..84].try_into()?) as u64;
        84 + n * 50 == bytes.len() as u64
    } else {
        false
    };
    let raw = if is_binary {
        parse_binary(&bytes)?
    } else {
        parse_ascii(
            std::str::from_utf8(&bytes)
                .map_err(|_| "invalid binary STL size or invalid ASCII text")?,
        )?
    };
    if raw.is_empty() {
        return Err("STL has no triangles".into());
    }
    if raw.iter().flatten().any(|v| !v.is_finite()) {
        return Err("STL contains non-finite coordinates".into());
    }
    let mut usable = Vec::with_capacity(raw.len() / 3);
    let mut omitted_degenerate_triangles = 0;
    for triangle in raw.chunks_exact(3) {
        if exact_zero_area(triangle)? {
            omitted_degenerate_triangles += 1;
        } else {
            usable.push(triangle);
        }
    }
    if usable.is_empty() {
        return Err(format!("STL has no usable triangles after omitting {omitted_degenerate_triangles} degenerate triangles").into());
    }
    let mut min = [f64::INFINITY; 3];
    for p in usable.iter().flat_map(|triangle| triangle.iter()) {
        for i in 0..3 {
            min[i] = min[i].min(p[i]);
        }
    }
    let mut mesh = Mesh::default();
    let mut lookup = HashMap::<[u64; 3], u32>::new();
    for triangle in usable {
        let mut indices = [0; 3];
        for (i, point) in triangle.iter().enumerate() {
            let p = std::array::from_fn(|axis| (point[axis] - min[axis]) * millimeters_per_unit);
            if p.iter().any(|v| !v.is_finite()) {
                return Err("scaled STL exceeds finite coordinate range".into());
            }
            let key = p.map(|v| if v == 0. { 0 } else { v.to_bits() });
            indices[i] = *lookup.entry(key).or_insert_with(|| {
                let index = mesh.positions.len() as u32;
                mesh.positions.push(p);
                index
            });
        }
        if indices[0] == indices[1] || indices[1] == indices[2] || indices[0] == indices[2] {
            return Err("nonzero STL triangle collapsed during rebasing/unit conversion".into());
        }
        let a = mesh.positions[indices[0] as usize];
        let b = mesh.positions[indices[1] as usize];
        let c = mesh.positions[indices[2] as usize];
        if exact_zero_area(&[a, b, c])? {
            return Err("nonzero STL triangle collapsed during rebasing/unit conversion".into());
        }
        mesh.triangles.push(indices);
    }
    mesh.bounds()?;
    Ok(StlImport {
        mesh,
        omitted_degenerate_triangles,
    })
}

pub fn load_stl(path: &Path, millimeters_per_unit: f64) -> Result<Mesh> {
    Ok(load_stl_with_report(path, millimeters_per_unit)?.mesh)
}

// No area tolerance: tiny nonzero facets are retained. Power-of-two scaling avoids
// area underflow/overflow for uniformly tiny/large triangles; FMA residuals keep
// rounded equal products from being mistaken for exact collinearity.
fn exact_zero_area(triangle: &[[f64; 3]]) -> Result<bool> {
    let u = std::array::from_fn::<_, 3, _>(|i| triangle[1][i] - triangle[0][i]);
    let v = std::array::from_fn::<_, 3, _>(|i| triangle[2][i] - triangle[0][i]);
    if u.iter().chain(v.iter()).any(|x| !x.is_finite()) {
        return Err("STL edge exceeds finite precision range".into());
    }
    if u.iter().all(|&x| x == 0.) || v.iter().all(|&x| x == 0.) {
        return Ok(true);
    }
    let largest = u.iter().chain(v.iter()).fold(0f64, |a, &b| a.max(b.abs()));
    let exponent = ((largest.to_bits() >> 52) & 0x7ff) as i32 - 1023;
    let scale = 2f64.powi(exponent);
    let us = u.map(|x| x / scale);
    let vs = v.map(|x| x / scale);
    if u.iter()
        .zip(us)
        .chain(v.iter().zip(vs))
        .any(|(&original, scaled)| original != 0. && scaled == 0.)
    {
        return Err("STL triangle spans an unsupported floating-point precision range".into());
    }
    for (i, j) in [(0, 1), (0, 2), (1, 2)] {
        let p = us[i] * vs[j];
        let q = us[j] * vs[i];
        if (p == 0. && us[i] != 0. && vs[j] != 0.) || (q == 0. && us[j] != 0. && vs[i] != 0.) {
            return Err("STL triangle area is below supported floating-point precision".into());
        }
        if p != q || us[i].mul_add(vs[j], -p) != us[j].mul_add(vs[i], -q) {
            return Ok(false);
        }
    }
    Ok(true)
}

fn parse_binary(bytes: &[u8]) -> Result<Vec<[f64; 3]>> {
    let count = u32::from_le_bytes(bytes[80..84].try_into()?) as usize;
    if count > MAX_VERTICES / 3 {
        return Err("STL exceeds triangle input budget".into());
    }
    let mut vertices = Vec::with_capacity(count * 3);
    for face in bytes[84..].chunks_exact(50) {
        for p in face[12..48].chunks_exact(12) {
            vertices.push([
                f32::from_le_bytes(p[0..4].try_into()?) as f64,
                f32::from_le_bytes(p[4..8].try_into()?) as f64,
                f32::from_le_bytes(p[8..12].try_into()?) as f64,
            ]);
        }
    }
    Ok(vertices)
}

fn parse_ascii(text: &str) -> Result<Vec<[f64; 3]>> {
    let mut lines = text.lines().map(str::trim).filter(|s| !s.is_empty());
    if lines
        .next()
        .is_none_or(|s| s.split_whitespace().next() != Some("solid"))
    {
        return Err("ASCII STL must begin with solid".into());
    }
    let mut vertices = Vec::new();
    let mut ended = false;
    while let Some(line) = lines.next() {
        if line.split_whitespace().next() == Some("endsolid") {
            ended = true;
            break;
        }
        let words = line.split_whitespace().collect::<Vec<_>>();
        if words.len() != 5 || words[..2] != ["facet", "normal"] {
            return Err("expected STL facet normal".into());
        }
        for n in &words[2..] {
            if !n.parse::<f64>()?.is_finite() {
                return Err("non-finite STL normal".into());
            }
        }
        if !lines
            .next()
            .is_some_and(|s| s.split_whitespace().eq(["outer", "loop"]))
        {
            return Err("expected STL outer loop".into());
        }
        for _ in 0..3 {
            let words = lines
                .next()
                .ok_or("missing STL vertex")?
                .split_whitespace()
                .collect::<Vec<_>>();
            if words.len() != 4 || words[0] != "vertex" {
                return Err("expected three-coordinate STL vertex".into());
            }
            vertices.push([words[1].parse()?, words[2].parse()?, words[3].parse()?]);
            if vertices.len() > MAX_VERTICES {
                return Err("STL exceeds vertex input budget".into());
            }
        }
        if lines.next() != Some("endloop") || lines.next() != Some("endfacet") {
            return Err("expected STL endloop/endfacet".into());
        }
    }
    if !ended || lines.next().is_some() {
        return Err("ASCII STL missing endsolid or has trailing data".into());
    }
    Ok(vertices)
}
