use crate::Result;
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{BufReader, BufWriter, Read, Write},
    path::Path,
};

pub const MAX_VERTICES: usize = 2_000_000;
pub const MAX_TRIANGLES: usize = 4_000_000;
const HEADER_BYTES: u64 = 20;

struct SnapshotDigest(Sha256);
impl Write for SnapshotDigest {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[derive(Clone, Debug, Default)]
pub struct Mesh {
    pub positions: Vec<[f64; 3]>,
    pub triangles: Vec<[u32; 3]>,
}

#[derive(Clone, Copy, Debug)]
pub struct Bounds {
    pub min: [f64; 3],
    pub max: [f64; 3],
}

impl Mesh {
    pub fn bounds(&self) -> Result<Bounds> {
        check_counts(self.positions.len(), self.triangles.len())?;
        let mut bounds = Bounds {
            min: [f64::INFINITY; 3],
            max: [f64::NEG_INFINITY; 3],
        };
        for p in &self.positions {
            for (i, value) in p.iter().copied().enumerate() {
                if !value.is_finite() {
                    return Err("mesh contains a non-finite position".into());
                }
                bounds.min[i] = bounds.min[i].min(value);
                bounds.max[i] = bounds.max[i].max(value);
            }
        }
        for tri in &self.triangles {
            if tri.iter().any(|&i| i as usize >= self.positions.len()) {
                return Err("mesh triangle index is out of range".into());
            }
        }
        Ok(bounds)
    }

    pub fn write_binary(&self, path: &Path) -> Result<()> {
        self.validate_binary()?;
        let mut out = BufWriter::new(File::create(path)?);
        self.write_snapshot(&mut out)?;
        out.flush()?;
        Ok(())
    }

    /// Hash the exact TNMESH01 bytes without allocating another mesh-sized buffer.
    pub fn snapshot_hash(&self) -> Result<String> {
        self.validate_binary()?;
        let mut digest = SnapshotDigest(Sha256::new());
        {
            let mut out = BufWriter::new(&mut digest);
            self.write_snapshot(&mut out)?;
            out.flush()?;
        }
        Ok(format!("{:x}", digest.0.finalize()))
    }

    fn validate_binary(&self) -> Result<()> {
        self.bounds()?;
        if self
            .positions
            .iter()
            .flatten()
            .any(|&v| !(v as f32).is_finite())
        {
            return Err("mesh coordinates exceed f32 snapshot range".into());
        }
        self.validate_snapshot_precision()
    }

    fn write_snapshot(&self, out: &mut impl Write) -> Result<()> {
        out.write_all(b"TNMESH01")?;
        for value in [1, self.positions.len() as u32, self.triangles.len() as u32] {
            out.write_all(&value.to_le_bytes())?;
        }
        for p in &self.positions {
            for v in p {
                out.write_all(&(*v as f32).to_le_bytes())?;
            }
        }
        for t in &self.triangles {
            for v in t {
                out.write_all(&v.to_le_bytes())?;
            }
        }
        Ok(())
    }

    pub fn read_binary(path: &Path) -> Result<Self> {
        let file = File::open(path)?;
        let length = file.metadata()?.len();
        let mut input = BufReader::new(file);
        let mut header = [0u8; 20];
        input.read_exact(&mut header)?;
        if &header[..8] != b"TNMESH01" || u32::from_le_bytes(header[8..12].try_into()?) != 1 {
            return Err("unsupported mesh magic or version".into());
        }
        let nv = u32::from_le_bytes(header[12..16].try_into()?) as usize;
        let nt = u32::from_le_bytes(header[16..20].try_into()?) as usize;
        check_counts(nv, nt)?;
        if length != HEADER_BYTES + (nv as u64 + nt as u64) * 12 {
            return Err("mesh snapshot size does not match counts".into());
        }
        let mut mesh = Mesh {
            positions: Vec::with_capacity(nv),
            triangles: Vec::with_capacity(nt),
        };
        let mut bytes = [0u8; 4];
        for _ in 0..nv {
            let mut p = [0.; 3];
            for v in &mut p {
                input.read_exact(&mut bytes)?;
                *v = f32::from_le_bytes(bytes) as f64;
            }
            mesh.positions.push(p);
        }
        for _ in 0..nt {
            let mut t = [0; 3];
            for v in &mut t {
                input.read_exact(&mut bytes)?;
                *v = u32::from_le_bytes(bytes);
            }
            mesh.triangles.push(t);
        }
        mesh.bounds()?;
        mesh.validate_snapshot_precision()?;
        Ok(mesh)
    }

    fn validate_snapshot_precision(&self) -> Result<()> {
        for triangle in &self.triangles {
            let a = self.positions[triangle[0] as usize].map(|v| v as f32 as f64);
            let b = self.positions[triangle[1] as usize].map(|v| v as f32 as f64);
            let c = self.positions[triangle[2] as usize].map(|v| v as f32 as f64);
            let ab = std::array::from_fn::<_, 3, _>(|i| b[i] - a[i]);
            let ac = std::array::from_fn::<_, 3, _>(|i| c[i] - a[i]);
            let cross = [
                ab[1] * ac[2] - ab[2] * ac[1],
                ab[2] * ac[0] - ab[0] * ac[2],
                ab[0] * ac[1] - ab[1] * ac[0],
            ];
            if cross.iter().all(|&v| v == 0.) {
                return Err("mesh has a degenerate triangle or loses geometry at f32 snapshot precision; rebase the input closer to the origin".into());
            }
        }
        Ok(())
    }
}

pub(crate) fn check_counts(vertices: usize, triangles: usize) -> Result<()> {
    if vertices == 0 || triangles == 0 {
        return Err("mesh has no usable geometry".into());
    }
    if vertices > MAX_VERTICES || triangles > MAX_TRIANGLES {
        return Err(format!("mesh budget exceeded: {vertices} vertices, {triangles} triangles (limits {MAX_VERTICES}/{MAX_TRIANGLES})").into());
    }
    Ok(())
}
