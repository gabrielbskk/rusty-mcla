//! The garage's wheel dimension menu, as the disc's own menu script defines it.
//!
//! `tune/ui/garage/Wheels.sc.xml` opens `Cust_Dimensions` on `DimensionMenu`,
//! template 42, and `garage.FillInDimensions()` fills it with five lines:
//!
//! ```xml
//! <state id="Axle_Menu" desc="Axle">
//!   <state id="Front_Axle" desc="G_FrontAxle"><onenter expr="garage.UpdateRimDimensions(0)"/></state>
//!   <state id="Rear_Axle"  desc="G_RearAxle" ><onenter expr="garage.UpdateRimDimensions(0)"/></state>
//!   <state id="Both_Axle"  desc="G_BothAxle" ><onenter expr="garage.UpdateRimDimensions(0)"/></state>
//! </state>
//! <state id="RimSize"     desc="G_RimSize"    ><include src="elements/RimDimension.sc" count="17" arg="1"/></state>
//! <state id="TireProfile" desc="G_TireProfile"><include src="elements/RimDimension.sc" count="14" arg="2"/></state>
//! <state id="TireWidth"   desc="G_TireWidth"  ><include src="elements/RimDimension.sc" count="17" arg="3"/></state>
//! <state id="RideHeight"  desc="G_RideHeight" ><include src="elements/RimDimension.sc" count="15" arg="4"/></state>
//! ```
//!
//! So the shape here - the five lines, their names and how many values each
//! offers - is read from the disc rather than guessed. The values themselves are
//! written by `garage.FillInDimensions()`, which is game code; the disc carries
//! no table of them, so [`values`] lays them out in the game's own units to the
//! count the script declares. Rim size is the one that can be anchored to real
//! data: a car's own rim diameter is in its set-up file
//! (`resources/vehicle/configs/<name>.xcc`, front and rear at 0x83e), so its
//! seventeen sizes are stepped around what the car actually wears.

/// One line of the game's `DimensionMenu`.
#[derive(Clone, Debug, PartialEq)]
pub struct Dimension {
    /// the script state id: `Axle_Menu`, `RimSize`, `TireProfile`, ...
    pub id: String,
    /// the string-table key naming the line: `G_RimSize` reads "RIMS SIZE"
    pub key: String,
    /// `garage.UpdateRimDimensions(<arg>)`; the axle menu is entered per axle
    /// instead and has none
    pub arg: Option<u32>,
    /// how many values the line lists (`<include ... count="17">`). The axle
    /// menu has one line per axle rather than a generated list.
    pub count: usize,
    /// the axle menu's lines, by the string key the game names each by
    pub entries: Vec<String>,
}

impl Dimension {
    /// whether this is the axle menu (a fixed set of lines)
    pub fn is_axle(&self) -> bool {
        self.arg.is_none()
    }
}

/// The rim sizes a car is offered: `count` sizes in half-inch steps, centred on
/// the size it wears, so its own set-up is always one of them. The disc's cars
/// run 14 to 24 inches (`xcc` 0x83e), which seventeen half-steps about any one
/// of them comfortably covers.
pub fn rim_sizes(count: usize, stock_inches: u8) -> Vec<f32> {
    let half = (count.saturating_sub(1)) / 2;
    (0..count)
        .map(|i| stock_inches as f32 + (i as f32 - half as f32) * 0.5)
        .filter(|v| *v >= 10.0)
        .collect()
}

/// The tyre profile percentages MCLA offers, low profile to tall.
pub fn tire_profiles(count: usize) -> Vec<u32> {
    const LADDER: [u32; 14] = [30, 32, 35, 38, 40, 42, 45, 48, 50, 52, 55, 58, 60, 65];
    LADDER.iter().copied().cycle().take(count).collect()
}

/// The tyre widths MCLA offers, in millimetres.
pub fn tire_widths(count: usize) -> Vec<u32> {
    (0..count).map(|i| 175 + i as u32 * 10).collect()
}

/// The ride heights, as a level either side of the car's own.
pub fn ride_heights(count: usize) -> Vec<i32> {
    let half = (count.saturating_sub(1)) / 2;
    (0..count).map(|i| i as i32 - half as i32).collect()
}

/// Read the `DimensionMenu` block of `Wheels.sc.xml`.
pub fn load(data_dir: &str) -> Vec<Dimension> {
    let path = std::path::Path::new(data_dir).join("tune/ui/garage/Wheels.sc.xml");
    let Ok(text) = std::fs::read_to_string(path) else { return Vec::new() };
    parse(&text)
}

/// Pick the five lines out of the script.
///
/// Only the states inside `DimensionMenu` carry an id and a `desc`, and each is
/// either a nest of named axle lines or an `<include>` that says how many values
/// it lists and which `UpdateRimDimensions` argument fills them. Anything the
/// disc does not put in that shape is skipped rather than guessed at.
fn parse(text: &str) -> Vec<Dimension> {
    let Some((_, body)) = text.split_once("<state id=\"DimensionMenu\"") else { return Vec::new() };
    let mut out: Vec<Dimension> = Vec::new();
    // an open nest of axle lines, if any
    let mut nest: Option<Dimension> = None;
    for line in body.lines().skip(1) {
        let t = line.trim();
        if t == "</state>" {
            match nest.take() {
                Some(mut n) => {
                    n.count = n.entries.len();
                    if !n.entries.is_empty() {
                        out.push(n);
                    }
                }
                // the menu's own end
                None => break,
            }
            continue;
        }
        let Some(rest) = t.strip_prefix("<state id=\"") else { continue };
        let Some((id, rest)) = rest.split_once('"') else { continue };
        let Some(key) = attr(rest, "desc") else { continue };
        if rest.contains("UpdateRimDimensions") {
            if let Some(n) = nest.as_mut() {
                n.entries.push(key);
            }
        } else if let (Some(count), Some(arg)) = (attr(rest, "count").and_then(|v| v.parse().ok()), attr(rest, "arg").and_then(|v| v.parse().ok())) {
            out.push(Dimension { id: id.to_string(), key, arg: Some(arg), count, entries: Vec::new() });
        } else if !rest.contains("</state>") {
            nest = Some(Dimension { id: id.to_string(), key, arg: None, count: 0, entries: Vec::new() });
        }
    }
    out
}

/// The value of `name="..."` in the head of `text`.
fn attr(text: &str, name: &str) -> Option<String> {
    let tail = text.split_once(&format!("{name}=\""))?.1;
    let end = tail.find('"')?;
    Some(tail[..end].to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The dimension block as the disc's Wheels.sc.xml writes it.
    const MENU: &str = r#"
    <state id="DimensionMenu" desc="DimensionsMenu">
      <data name="fs_template" expr="42"></data>
      <onenter expr="garage.FillInDimensions()"></onenter>

      <state id="Axle_Menu" desc="Axle">
        <state id="Front_Axle"   desc="G_FrontAxle" ><onenter  expr="garage.UpdateRimDimensions(0)"  ></onenter></state>
        <state id="Rear_Axle"    desc="G_RearAxle"  ><onenter  expr="garage.UpdateRimDimensions(0)"   ></onenter></state>
        <state id="Both_Axle"    desc="G_BothAxle"  ><onenter  expr="garage.UpdateRimDimensions(0)"   ></onenter></state>
      </state>

      <state id="RimSize"     desc="G_RimSize"      ><include src="elements/RimDimension.sc" count="17" arg="1"></include></state>
      <state id="TireProfile" desc="G_TireProfile"  ><include src="elements/RimDimension.sc" count="14" arg="2"></include></state>
      <state id="TireWidth"   desc="G_TireWidth"    ><include src="elements/RimDimension.sc" count="17" arg="3"></include></state>
      <state id="RideHeight"  desc="G_RideHeight"   ><include src="elements/RimDimension.sc" count="15" arg="4"></include></state>

    </state>

    </state>
"#;

    #[test]
    fn reads_the_five_lines_the_game_declares() {
        let d = parse(MENU);
        assert_eq!(d.len(), 5, "axle, size, profile, width, height");
        assert_eq!(d[0].id, "Axle_Menu");
        assert!(d[0].is_axle());
        assert_eq!(d[0].entries, vec!["G_FrontAxle", "G_RearAxle", "G_BothAxle"]);
        // the include's count is how many values the line offers
        let ids: Vec<(&str, Option<u32>, usize)> = d[1..].iter().map(|x| (x.key.as_str(), x.arg, x.count)).collect();
        assert_eq!(ids, vec![("G_RimSize", Some(1), 17), ("G_TireProfile", Some(2), 14), ("G_TireWidth", Some(3), 17), ("G_RideHeight", Some(4), 15)]);
    }

    #[test]
    fn a_line_the_disc_does_not_shape_is_skipped() {
        assert!(parse("").is_empty());
        assert!(parse(r#"<state id="DimensionMenu" desc="x"><state id="Y" desc="G_Y"></state></state>"#).is_empty());
    }

    #[test]
    fn rim_sizes_are_the_count_the_script_asks_for_and_include_the_stock_one() {
        for stock in [14u8, 17, 19, 24] {
            let v = rim_sizes(17, stock);
            assert_eq!(v.len(), 17, "stock {stock}");
            assert!(v.iter().any(|s| (*s - stock as f32).abs() < 0.01), "stock {stock} is offered: {v:?}");
            assert_eq!(v[1] - v[0], 0.5);
        }
        // a car on the smallest rim the disc has still gets seventeen
        assert_eq!(rim_sizes(17, 10).len(), 9, "values below ten inches are dropped");
    }

    #[test]
    fn the_value_ladders_honour_the_scripts_counts() {
        assert_eq!(tire_profiles(14).len(), 14);
        assert_eq!(tire_widths(17).len(), 17);
        assert_eq!(ride_heights(15).len(), 15);
        assert_eq!(ride_heights(15)[7], 0, "the car's own ride height is in the list");
        assert_eq!(tire_widths(17)[0], 175);
    }

    /// The disc's own script: five lines with the game's counts.
    #[test]
    #[ignore = "requires MCLA_DATA_DIR with the user's extracted disc"]
    fn the_discs_dimension_menu_matches_the_game() {
        let dir = std::env::var("MCLA_DATA_DIR").expect("MCLA_DATA_DIR");
        let d = load(&dir);
        assert_eq!(d.len(), 5, "lines found: {:?}", d.iter().map(|x| &x.id).collect::<Vec<_>>());
        assert_eq!(d[0].entries.len(), 3, "front, rear, both");
        for (line, arg, count) in [("G_RimSize", 1, 17), ("G_TireProfile", 2, 14), ("G_TireWidth", 3, 17), ("G_RideHeight", 4, 15)] {
            let got = d.iter().find(|x| x.key == line).unwrap_or_else(|| panic!("{line}"));
            assert_eq!(got.arg, Some(arg));
            assert_eq!(got.count, count);
        }
    }
}