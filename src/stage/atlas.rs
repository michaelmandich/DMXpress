//! The gobo mask atlas: every gobo any patched fixture can show, decoded
//! once and handed to the GPU as one 2D texture array. Beams name their
//! gobos by layer index, so switching slots mid-show never uploads anything.

use std::collections::HashMap;
use std::sync::Arc;

use crate::gobo::{Catalogue, Mask, MASK_SIZE};
use crate::showbuddy::Patch;

/// What the GPU side needs to (re)build the texture array.
pub(crate) struct AtlasUpload {
    /// Bumped on every rebuild; the GPU compares it to what it holds.
    pub generation: u64,
    pub size: u32,
    pub layers: Vec<Arc<Mask>>,
}

#[derive(Default)]
pub(crate) struct GoboAtlas {
    /// Every key the patch asked for, found or not, so a missing mask is
    /// not retried every frame.
    keys: Vec<String>,
    index: HashMap<String, u32>,
    generation: u64,
    upload: Option<Arc<AtlasUpload>>,
}

impl GoboAtlas {
    /// Texture arrays this deep are still tiny (64 KB a layer).
    pub const MAX_LAYERS: usize = 512;

    /// Bring the atlas in line with the patch: every gobo key on any band,
    /// in a stable order. Cheap when nothing changed.
    pub fn sync(&mut self, patch: &Patch, catalogue: &Catalogue) {
        let mut keys: Vec<&str> = patch
            .fixtures
            .iter()
            .flat_map(|f| f.channels.iter())
            .flat_map(|c| c.bands.iter())
            .filter_map(|b| b.gobo.as_deref())
            .collect();
        keys.sort_unstable();
        keys.dedup();
        keys.truncate(Self::MAX_LAYERS);
        if keys.len() == self.keys.len() && keys.iter().zip(&self.keys).all(|(a, b)| *a == b) {
            return;
        }
        let mut layers = Vec::with_capacity(keys.len());
        let mut index = HashMap::with_capacity(keys.len());
        for key in &keys {
            if let Some(mask) = catalogue.mask(key) {
                index.insert((*key).to_string(), layers.len() as u32);
                layers.push(mask);
            }
        }
        self.keys = keys.into_iter().map(str::to_string).collect();
        self.index = index;
        self.generation += 1;
        self.upload = Some(Arc::new(AtlasUpload { generation: self.generation, size: MASK_SIZE, layers }));
    }

    pub fn layer(&self, key: &str) -> Option<u32> {
        self.index.get(key).copied()
    }

    pub fn upload(&self) -> Option<Arc<AtlasUpload>> {
        self.upload.clone()
    }
}
