//! A whole model: root geometry, component definitions, materials and tags.

use slotmap::SlotMap;

use crate::entities::{DefinitionId, Entities};

#[derive(Debug, Clone, PartialEq)]
pub struct Material {
    pub name: String,
    /// sRGB color.
    pub color: [u8; 3],
    /// 1 = opaque.
    pub opacity: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Layer {
    pub name: String,
    pub color: [u8; 3],
    pub visible: bool,
}

#[derive(Debug, Clone, Default)]
pub struct Definition {
    pub name: String,
    pub guid: String,
    pub entities: Entities,
}

#[derive(Debug, Clone)]
pub struct Model {
    pub root: Entities,
    pub definitions: SlotMap<DefinitionId, Definition>,
    pub materials: Vec<Material>,
    /// Index 0 is `Layer0` (untagged).
    pub layers: Vec<Layer>,
    /// Style colors for faces without a material.
    pub front_color: [u8; 3],
    pub back_color: [u8; 3],
}

impl Default for Model {
    fn default() -> Self {
        Self {
            root: Entities::new(),
            definitions: SlotMap::with_key(),
            materials: Vec::new(),
            layers: vec![Layer {
                name: "Layer0".into(),
                color: [255, 255, 255],
                visible: true,
            }],
            // SketchUp's default style (values from OpenSKP's fixture checks).
            front_color: [255, 255, 255],
            back_color: [164, 178, 187],
        }
    }
}
