//! `Section::palette_index` bit-packing: the two on-disk layouts must decode
//! identically.
//!
//! 1.13–1.15 pack palette indices contiguously, so an entry can straddle a
//! long boundary. 1.16+ pad every long instead. The layouts only differ when
//! `bits` does not divide 64 (bits 5, 6, 7), and no save on this machine
//! exercises the contiguous one, so it is covered with synthetic data here.
//!
//! These tests drive the real `Section`/`BlockStates` types, so they fail if
//! production decoding regresses rather than verifying a copy of it.

use world_viewer_lib::testing::{BlockStates, Section};

/// Pack 4096 indices contiguously across long boundaries (1.13–1.15).
fn pack_contiguous(indices: &[u16], bits: u32) -> Vec<u64> {
    let total_bits = indices.len() * bits as usize;
    let mut data = vec![0u64; total_bits.div_ceil(64)];
    for (i, &v) in indices.iter().enumerate() {
        let bit = i * bits as usize;
        let long = bit / 64;
        let offset = bit % 64;
        data[long] |= (v as u64) << offset;
        if offset + bits as usize > 64 {
            data[long + 1] |= (v as u64) >> (64 - offset);
        }
    }
    data
}

/// Pack 4096 indices padded to a long boundary (1.16+).
fn pack_padded(indices: &[u16], bits: u32) -> Vec<u64> {
    let per_long = 64 / bits as usize;
    let mut data = vec![0u64; indices.len().div_ceil(per_long)];
    for (i, &v) in indices.iter().enumerate() {
        let long = i / per_long;
        let offset = (i % per_long) * bits as usize;
        data[long] |= (v as u64) << offset;
    }
    data
}

/// A palette large enough to force the requested bits per entry.
fn palette(n: usize) -> Vec<String> {
    (0..n).map(|i| format!("minecraft:block_{}", i)).collect()
}

fn section_with(indices: &[u16], entries: usize, bits: u32, contiguous: bool) -> Section {
    let data = if contiguous {
        pack_contiguous(indices, bits)
    } else {
        pack_padded(indices, bits)
    };
    Section {
        y: 0,
        blocks16: None,
        blocks: None,
        data16: None,
        data: None,
        add: None,
        block_states: Some(BlockStates::from_parts(palette(entries), Some(data))),
    }
}

/// Every block index decoded through the production path must match the input.
fn assert_decodes(section: &Section, indices: &[u16]) {
    for (i, &want) in indices.iter().enumerate() {
        let (x, y, z) = (i % 16, (i / 256) % 16, (i / 16) % 16);
        assert_eq!(
            section.palette_index(x, y, z),
            Some(want as usize),
            "mismatch at block index {}",
            i
        );
    }
}

#[test]
fn contiguous_layout_is_detected_and_decoded() {
    // 22 entries -> 5 bits, which does not divide 64, so entries straddle.
    let indices: Vec<u16> = (0..4096).map(|i| (i % 22) as u16).collect();
    let section = section_with(&indices, 22, 5, true);
    let bs = section.block_states.as_ref().unwrap();

    assert_eq!(bs.bits, 5);
    assert!(
        bs.spans,
        "contiguous array must be detected as the spanning layout"
    );
    assert_decodes(&section, &indices);
}

#[test]
fn padded_layout_is_detected_and_decoded() {
    let indices: Vec<u16> = (0..4096).map(|i| (i % 22) as u16).collect();
    let section = section_with(&indices, 22, 5, false);
    let bs = section.block_states.as_ref().unwrap();

    assert_eq!(bs.bits, 5);
    assert!(!bs.spans, "padded array must not be detected as spanning");
    assert_decodes(&section, &indices);
}

#[test]
fn both_layouts_agree_on_the_same_indices() {
    let indices: Vec<u16> = (0..4096).map(|i| ((i * 7) % 40) as u16).collect();
    let contiguous = section_with(&indices, 40, 6, true);
    let padded = section_with(&indices, 40, 6, false);

    assert!(contiguous.block_states.as_ref().unwrap().spans);
    assert!(!padded.block_states.as_ref().unwrap().spans);

    assert_decodes(&contiguous, &indices);
    assert_decodes(&padded, &indices);
}

/// When `bits` divides 64 both layouts coincide, so detection is moot.
#[test]
fn four_bit_layouts_are_identical() {
    let indices: Vec<u16> = (0..4096).map(|i| (i % 9) as u16).collect();
    let a = section_with(&indices, 9, 4, true);
    let b = section_with(&indices, 9, 4, false);
    assert_eq!(
        a.block_states.as_ref().unwrap().spans,
        b.block_states.as_ref().unwrap().spans
    );
    assert_decodes(&a, &indices);
    assert_decodes(&b, &indices);
}

/// A single-entry palette carries no data: every block is palette[0].
#[test]
fn single_entry_palette_needs_no_data() {
    let section = Section {
        y: 0,
        blocks16: None,
        blocks: None,
        data16: None,
        data: None,
        add: None,
        block_states: Some(BlockStates::from_parts(palette(1), None)),
    };
    assert_eq!(section.block_states.as_ref().unwrap().bits, 0);
    assert_eq!(section.palette_index(3, 4, 5), Some(0));
}
