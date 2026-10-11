//! KMZ export (Google Earth): a ZIP holding `doc.kml`, the model as COLLADA in
//! `models/<name>.dae` (from [`crate::mesh_export::to_dae`]) and its textures next to
//! it. The KML places the model at the model's location (Model Info > Geo-location,
//! the shadow latitude and longitude) on the ground, turned by the north angle.
//! The KML 2.2 layout follows the OGC KML specification and Google's KML reference.

use std::fmt::Write as _;

use crate::mesh_export::{self, DaeOptions};
use crate::model::Model;

/// Encodes the model as KMZ. `name` is the placemark's name and the DAE's file stem.
pub fn to_kmz(model: &Model, name: &str, two_sided: bool) -> Result<Vec<u8>, String> {
    let stem: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let stem = if stem.is_empty() {
        "model".into()
    } else {
        stem
    };
    let dae = mesh_export::to_dae(
        model,
        &DaeOptions {
            two_sided,
            base_name: stem.clone(),
        },
    );
    let s = &model.shadows;
    let mut kml = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    kml.push_str("<kml xmlns=\"http://www.opengis.net/kml/2.2\">\n  <Placemark>\n");
    let _ = writeln!(kml, "    <name>{}</name>", super::xml_escape(name));
    kml.push_str("    <Model>\n      <altitudeMode>relativeToGround</altitudeMode>\n");
    let _ = writeln!(
        kml,
        "      <Location><longitude>{}</longitude><latitude>{}</latitude><altitude>0</altitude></Location>",
        s.longitude, s.latitude
    );
    let _ = writeln!(
        kml,
        "      <Orientation><heading>{}</heading><tilt>0</tilt><roll>0</roll></Orientation>",
        super::num(model.geo.north_angle)
    );
    kml.push_str("      <Scale><x>1</x><y>1</y><z>1</z></Scale>\n");
    let _ = writeln!(kml, "      <Link><href>models/{stem}.dae</href></Link>");
    if !dae.textures.is_empty() {
        kml.push_str("      <ResourceMap>\n");
        for (file, _) in &dae.textures {
            let f = super::xml_escape(file);
            let _ = writeln!(
                kml,
                "        <Alias><targetHref>models/{f}</targetHref><sourceHref>{f}</sourceHref></Alias>"
            );
        }
        kml.push_str("      </ResourceMap>\n");
    }
    kml.push_str("    </Model>\n  </Placemark>\n</kml>\n");
    let mut files = vec![
        ("doc.kml".to_string(), kml.into_bytes()),
        (format!("models/{stem}.dae"), dae.dae.into_bytes()),
    ];
    for (file, data) in &dae.textures {
        files.push((format!("models/{file}"), data.to_vec()));
    }
    super::threemf::write_zip(&files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn kmz_holds_kml_and_dae() {
        let m = crate::formats::tests::painted_cube();
        let bytes = to_kmz(&m, "My Box", true).unwrap();
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&bytes[..])).unwrap();
        let mut kml = String::new();
        zip.by_name("doc.kml")
            .unwrap()
            .read_to_string(&mut kml)
            .unwrap();
        assert!(kml.contains("<href>models/My_Box.dae</href>"));
        assert!(kml.contains("<name>My Box</name>"));
        let mut dae = String::new();
        zip.by_name("models/My_Box.dae")
            .unwrap()
            .read_to_string(&mut dae)
            .unwrap();
        assert!(dae.contains("<COLLADA"));
    }
}
