//! The game's ambient population tables (`tune/ambients`): a district map
//! (`sc.dds`, one grey level per district), a tree of `mcAmbientDistribution.xml` files
//! listing which vehicle / pedestrian types appear in every district at which time of
//! day, and the overall density over the day. One tree holds traffic, the other
//! pedestrians.

use glam::Vec3;
use hashbrown::HashMap;
use std::path::Path;

pub fn xml_value(text: &str, tag: &str) -> Option<f32> {
    let i = text.find(&format!("<{tag} "))?;
    let rest = &text[i..];
    let v = rest.find("value=\"")? + 7;
    let e = rest[v..].find('"')?;
    rest[v..v + e].trim().parse().ok()
}

/// Weight of something over the day: (start hour, end hour, weight) rows of a `TodArray`.
#[derive(Default, Clone, Debug)]
pub struct Tod(Vec<(f32, f32, f32)>);

impl Tod {
    pub fn parse(xml: &str) -> Tod {
        let mut v = Vec::new();
        for item in xml.split("<Item>").skip(1) {
            let (Some(a), Some(b), Some(w)) = (xml_value(item, "iStartTime"), xml_value(item, "iEndTime"), xml_value(item, "fWeight")) else { continue };
            // times are written as HHMM
            v.push((a / 100.0, if b <= a { b / 100.0 + 24.0 } else { b / 100.0 }, w));
        }
        Tod(v)
    }

    pub fn at(&self, hour: f32) -> f32 {
        if self.0.is_empty() {
            return 1.0;
        }
        self.0.iter().find(|(a, b, _)| (hour >= *a && hour < *b) || (hour + 24.0 >= *a && hour + 24.0 < *b)).map(|r| r.2).unwrap_or(0.0)
    }
}

#[derive(Default)]
pub struct Distribution {
    map: Vec<u8>,
    /// per district colour: (type name, weight over the day), ancestors' lists included
    pub types: HashMap<u8, Vec<(String, Tod)>>,
    pub density: Tod,
}

fn type_list(xml: &str) -> Vec<(String, Tod)> {
    let body = xml.split("<TodDensityArray>").next().unwrap_or("");
    body.split("<TypeName>").skip(1).map(|item| (item.split('<').next().unwrap_or("").trim().to_ascii_lowercase(), Tod::parse(item))).collect()
}

impl Distribution {
    // world rectangle covered by the 256x256 map (fitted to the parked car lists, which
    // are stored per district)
    const X0: f32 = -3102.5;
    const X1: f32 = 2912.5;
    const Z0: f32 = -2450.0;
    const Z1: f32 = 2060.0;

    pub fn district(&self, p: Vec3) -> Option<u8> {
        if self.map.len() < 65536 {
            return None;
        }
        let u = ((p.x - Self::X0) / (Self::X1 - Self::X0) * 256.0).clamp(0.0, 255.0) as usize;
        let v = ((p.z - Self::Z0) / (Self::Z1 - Self::Z0) * 256.0).clamp(0.0, 255.0) as usize;
        Some(self.map[v * 256 + u])
    }

    pub fn loaded(&self) -> bool {
        !self.types.is_empty()
    }

    /// Load the tree whose type names start with `prefix` ("ped_" or "va_").
    pub fn load(data_dir: &Path, prefix: &str) -> Distribution {
        let mut d = Distribution::default();
        let amb = data_dir.join("tune/ambients");
        if let Ok(b) = std::fs::read(amb.join("sc.dds")).or_else(|_| std::fs::read(amb.join("LA.dds"))) {
            if b.len() >= 128 + 65536 {
                d.map = b[b.len() - 65536..].to_vec();
            }
        }
        let Ok(rd) = std::fs::read_dir(&amb) else { return d };
        let mut tops: Vec<_> = rd.flatten().map(|e| e.path()).collect();
        tops.sort();
        for top in tops {
            let Ok(root_xml) = std::fs::read_to_string(top.join("mcAmbientDistribution.xml")) else { continue };
            // walk the tree, handing every node the lists of its ancestors
            let mut leaves: Vec<(u8, Vec<(String, Tod)>)> = Vec::new();
            let mut stack = vec![(top.clone(), Vec::<(String, Tod)>::new())];
            while let Some((dir, inherited)) = stack.pop() {
                let mut list = inherited;
                if let Ok(x) = std::fs::read_to_string(dir.join("mcAmbientDistribution.xml")) {
                    list.extend(type_list(&x));
                }
                if let Ok(tree) = std::fs::read_to_string(dir.join("mcAmbientDistributionTreeData.xml")) {
                    if let Some(c) = xml_value(&tree, "iMapColor") {
                        leaves.push((c as u8, list.clone()));
                    }
                }
                if let Ok(rd) = std::fs::read_dir(&dir) {
                    for c in rd.flatten() {
                        if c.path().is_dir() {
                            stack.push((c.path(), list.clone()));
                        }
                    }
                }
            }
            if !leaves.iter().any(|(_, l)| l.iter().any(|(n, _)| n.starts_with(prefix))) {
                continue;
            }
            if let Some(i) = root_xml.find("<TodDensityArray>") {
                d.density = Tod::parse(&root_xml[i..]);
            }
            d.types = leaves.into_iter().collect();
            break;
        }
        d
    }

    /// Weighted pick for this place and hour among the types `usable` accepts
    /// (`r` in 0..1). `None` when the district has nothing to offer.
    pub fn pick(&self, pos: Vec3, hour: f32, r: f32, usable: &dyn Fn(&str) -> bool) -> Option<&str> {
        let list = self.types.get(&self.district(pos)?)?;
        let total: f32 = list.iter().filter(|(n, _)| usable(n)).map(|(_, t)| t.at(hour)).sum();
        if total <= 0.0 {
            return None;
        }
        let mut x = r.clamp(0.0, 0.9999) * total;
        let mut last = None;
        for (n, t) in list.iter().filter(|(n, _)| usable(n)) {
            let w = t.at(hour);
            if w <= 0.0 {
                continue;
            }
            last = Some(n.as_str());
            x -= w;
            if x <= 0.0 {
                break;
            }
        }
        last
    }
}

/// Parking spots flagged `bRandomSmallAmbient` in the per-district parked car lists
/// (`*_parked_ambients_AmbientData.xml`), keyed by position in decimetres.
pub fn parked_small_spots(data_dir: &Path) -> hashbrown::HashSet<(i32, i32)> {
    let mut out = hashbrown::HashSet::new();
    let amb = data_dir.join("tune/ambients");
    let Ok(rd) = std::fs::read_dir(&amb) else { return out };
    let mut seen = hashbrown::HashSet::new();
    for sub in rd.flatten().filter(|e| e.path().is_dir()) {
        let Ok(files) = std::fs::read_dir(sub.path()) else { continue };
        for f in files.flatten() {
            let name = f.file_name().to_string_lossy().to_string();
            if !name.ends_with("_AmbientData.xml") || !seen.insert(name) {
                continue;
            }
            let Ok(t) = std::fs::read_to_string(f.path()) else { continue };
            for item in t.split("<Item>").skip(1) {
                if !item.contains("bRandomSmallAmbient value=\"true\"") {
                    continue;
                }
                let Some(i) = item.find("<vVerts") else { continue };
                let Some(j) = item[i..].find('>') else { continue };
                let nums: Vec<f32> = item[i + j + 1..].split_whitespace().take(3).filter_map(|x| x.parse().ok()).collect();
                if nums.len() == 3 {
                    out.insert(((nums[0] * 10.0).round() as i32, (nums[2] * 10.0).round() as i32));
                }
            }
        }
    }
    out
}
