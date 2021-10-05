mod objects;
use objects::*;

pub struct BlockBuilder(pub Block);

impl BlockBuilder {
    /// Creates a new, empty builder
    pub fn new() -> BlockBuilder {
	BlockBuilder::default()
    }

    pub fn push(
