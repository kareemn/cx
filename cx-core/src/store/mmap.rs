use crate::error::CxError;
use crate::graph::csr::CsrGraph;
use crate::graph::edges::Edge;
use crate::graph::nodes::Node;
use crate::graph::string_interner::StringInterner;

/// Current graph file format version.
pub const FORMAT_VERSION: u32 = 2;

/// Magic bytes identifying a .cxgraph file.
pub const MAGIC: [u8; 4] = *b"CX01";

/// Page size for alignment (4KB).
const PAGE_SIZE: u64 = 4096;

/// Round up to next page boundary.
fn page_align(offset: u64) -> u64 {
    (offset + PAGE_SIZE - 1) & !(PAGE_SIZE - 1)
}

/// The first 64 bytes of the .cxgraph file. Validates format and locates all sections.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct GraphFileHeader {
    pub magic: [u8; 4],
    pub version: u32,
    pub node_count: u32,
    pub edge_count: u32,
    pub summary_node_count: u32,
    pub summary_edge_count: u32,
    pub string_table_size: u32,
    pub _pad0: [u8; 4],
    pub nodes_offset: u64,
    pub edges_offset: u64,
    pub strings_offset: u64,
    pub checksum: u32,
    pub _reserved: [u8; 4],
}

const _: () = assert!(std::mem::size_of::<GraphFileHeader>() == 64);

impl GraphFileHeader {
    /// Compute CRC32 checksum over the header fields (excluding the checksum field itself).
    pub fn compute_checksum(&self) -> u32 {
        let bytes = unsafe {
            std::slice::from_raw_parts(
                self as *const Self as *const u8,
                std::mem::size_of::<Self>(),
            )
        };
        // Hash everything before the checksum field (offset 56) and skip checksum + reserved
        let mut hasher = crc32fast::Hasher::new();
        hasher.update(&bytes[..56]);
        hasher.finalize()
    }
}

/// Write a CsrGraph to disk in the mmap-compatible format.
pub fn write_graph(graph: &CsrGraph, path: &std::path::Path) -> crate::Result<()> {
    use std::io::Write;

    let node_count = graph.nodes.len() as u32;
    let edge_count = graph.edges.len() as u32;
    let string_data = graph.strings.data();

    // Calculate offsets (page-aligned)
    let header_size = std::mem::size_of::<GraphFileHeader>() as u64;
    let nodes_offset = page_align(header_size);
    let nodes_size = (node_count as u64) * (std::mem::size_of::<Node>() as u64);

    let offsets_offset = nodes_offset + nodes_size;
    let offsets_size = (graph.offsets.len() as u64) * 4;

    let edges_offset = page_align(offsets_offset + offsets_size);
    let edges_size = (edge_count as u64) * (std::mem::size_of::<Edge>() as u64);

    let rev_offsets_offset = edges_offset + edges_size;
    let rev_offsets_size = (graph.rev_offsets.len() as u64) * 4;

    let rev_edges_offset = rev_offsets_offset + rev_offsets_size;
    let rev_edges_size = (graph.rev_edges.len() as u64) * (std::mem::size_of::<Edge>() as u64);

    let strings_offset = page_align(rev_edges_offset + rev_edges_size);

    // Build header
    let mut header = GraphFileHeader {
        magic: MAGIC,
        version: FORMAT_VERSION,
        node_count,
        edge_count,
        summary_node_count: 0,
        summary_edge_count: 0,
        string_table_size: string_data.len() as u32,
        _pad0: [0; 4],
        nodes_offset,
        edges_offset,
        strings_offset,
        checksum: 0,
        _reserved: [0; 4],
    };
    header.checksum = header.compute_checksum();

    let total_size = strings_offset + string_data.len() as u64;

    let mut file = std::fs::File::create(path)?;
    let mut buf = vec![0u8; total_size as usize];

    // Write header
    let header_bytes = unsafe {
        std::slice::from_raw_parts(
            &header as *const GraphFileHeader as *const u8,
            std::mem::size_of::<GraphFileHeader>(),
        )
    };
    buf[..header_bytes.len()].copy_from_slice(header_bytes);

    // Write nodes
    let nodes_bytes = unsafe {
        std::slice::from_raw_parts(
            graph.nodes.as_ptr() as *const u8,
            graph.nodes.len() * std::mem::size_of::<Node>(),
        )
    };
    let off = nodes_offset as usize;
    buf[off..off + nodes_bytes.len()].copy_from_slice(nodes_bytes);

    // Write forward offsets
    let offsets_bytes = unsafe {
        std::slice::from_raw_parts(
            graph.offsets.as_ptr() as *const u8,
            graph.offsets.len() * 4,
        )
    };
    let off = offsets_offset as usize;
    buf[off..off + offsets_bytes.len()].copy_from_slice(offsets_bytes);

    // Write forward edges
    let edges_bytes = unsafe {
        std::slice::from_raw_parts(
            graph.edges.as_ptr() as *const u8,
            graph.edges.len() * std::mem::size_of::<Edge>(),
        )
    };
    let off = edges_offset as usize;
    buf[off..off + edges_bytes.len()].copy_from_slice(edges_bytes);

    // Write reverse offsets
    let rev_offsets_bytes = unsafe {
        std::slice::from_raw_parts(
            graph.rev_offsets.as_ptr() as *const u8,
            graph.rev_offsets.len() * 4,
        )
    };
    let off = rev_offsets_offset as usize;
    buf[off..off + rev_offsets_bytes.len()].copy_from_slice(rev_offsets_bytes);

    // Write reverse edges
    let rev_edges_bytes = unsafe {
        std::slice::from_raw_parts(
            graph.rev_edges.as_ptr() as *const u8,
            graph.rev_edges.len() * std::mem::size_of::<Edge>(),
        )
    };
    let off = rev_edges_offset as usize;
    buf[off..off + rev_edges_bytes.len()].copy_from_slice(rev_edges_bytes);

    // Write string table
    let off = strings_offset as usize;
    buf[off..off + string_data.len()].copy_from_slice(string_data);

    file.write_all(&buf)?;
    file.sync_all()?;

    Ok(())
}

/// Load a CsrGraph from a memory-mapped .cxgraph file.
pub fn load_graph(path: &std::path::Path) -> crate::Result<CsrGraph> {
    let file = std::fs::File::open(path)?;
    let mmap = unsafe { memmap2::Mmap::map(&file) }
        .map_err(CxError::Io)?;

    load_graph_from_bytes(&mmap)
}

/// Load a CsrGraph from raw bytes (used by both mmap and testing).
pub fn load_graph_from_bytes(data: &[u8]) -> crate::Result<CsrGraph> {
    let header_size = std::mem::size_of::<GraphFileHeader>();

    if data.len() < header_size {
        return Err(CxError::CorruptGraph("file too small for header".into()));
    }

    // Read header
    let header: GraphFileHeader = unsafe {
        std::ptr::read_unaligned(data.as_ptr() as *const GraphFileHeader)
    };

    // Validate magic
    if header.magic != MAGIC {
        return Err(CxError::CorruptGraph(format!(
            "invalid magic bytes: {:?}",
            header.magic
        )));
    }

    // Validate version
    if header.version != FORMAT_VERSION {
        return Err(CxError::VersionMismatch {
            found: header.version,
            expected: FORMAT_VERSION,
        });
    }

    // Validate checksum
    let expected_checksum = header.compute_checksum();
    if header.checksum != expected_checksum {
        return Err(CxError::CorruptGraph("header checksum mismatch".into()));
    }

    let node_count = header.node_count as usize;
    let edge_count = header.edge_count as usize;
    let node_size = std::mem::size_of::<Node>();
    let edge_size = std::mem::size_of::<Edge>();

    // Validate that sections fit within the file
    let nodes_end = header.nodes_offset as usize + node_count * node_size;
    let offsets_end = nodes_end + (node_count + 1) * 4;
    let edges_end = header.edges_offset as usize + edge_count * edge_size;

    if nodes_end > data.len() || edges_end > data.len() || offsets_end > data.len() {
        return Err(CxError::CorruptGraph("section extends beyond file".into()));
    }

    let strings_end = header.strings_offset as usize + header.string_table_size as usize;
    if strings_end > data.len() {
        return Err(CxError::CorruptGraph(
            "string table extends beyond file".into(),
        ));
    }

    // Read nodes
    let nodes: Vec<Node> = {
        let off = header.nodes_offset as usize;
        let src = &data[off..off + node_count * node_size];
        let mut nodes = Vec::with_capacity(node_count);
        for i in 0..node_count {
            let node = unsafe {
                std::ptr::read_unaligned(src[i * node_size..].as_ptr() as *const Node)
            };
            nodes.push(node);
        }
        nodes
    };

    // Read forward offsets
    let offsets: Vec<u32> = {
        let off = header.nodes_offset as usize + node_count * node_size;
        let count = node_count + 1;
        let src = &data[off..off + count * 4];
        let mut offsets = Vec::with_capacity(count);
        for i in 0..count {
            let val = u32::from_le_bytes(src[i * 4..(i + 1) * 4].try_into().unwrap());
            offsets.push(val);
        }
        offsets
    };

    // Read forward edges
    let edges: Vec<Edge> = {
        let off = header.edges_offset as usize;
        let src = &data[off..off + edge_count * edge_size];
        let mut edges = Vec::with_capacity(edge_count);
        for i in 0..edge_count {
            let edge = unsafe {
                std::ptr::read_unaligned(src[i * edge_size..].as_ptr() as *const Edge)
            };
            edges.push(edge);
        }
        edges
    };

    // Read reverse offsets and edges (right after forward edges)
    let rev_offsets_start = header.edges_offset as usize + edge_count * edge_size;
    let rev_offsets: Vec<u32> = {
        let count = node_count + 1;
        let src = &data[rev_offsets_start..rev_offsets_start + count * 4];
        let mut offsets = Vec::with_capacity(count);
        for i in 0..count {
            let val = u32::from_le_bytes(src[i * 4..(i + 1) * 4].try_into().unwrap());
            offsets.push(val);
        }
        offsets
    };

    let rev_edges_start = rev_offsets_start + (node_count + 1) * 4;
    let rev_edges: Vec<Edge> = {
        let src = &data[rev_edges_start..rev_edges_start + edge_count * edge_size];
        let mut edges = Vec::with_capacity(edge_count);
        for i in 0..edge_count {
            let edge = unsafe {
                std::ptr::read_unaligned(src[i * edge_size..].as_ptr() as *const Edge)
            };
            edges.push(edge);
        }
        edges
    };

    // Read string table
    let string_data = {
        let off = header.strings_offset as usize;
        let size = header.string_table_size as usize;
        data[off..off + size].to_vec()
    };
    let strings = StringInterner::from_data(string_data);

    Ok(CsrGraph {
        nodes,
        edges,
        offsets,
        rev_edges,
        rev_offsets,
        strings,
    })
}

/// Issue mmap advisory hints to the OS about access patterns.
#[cfg(unix)]
pub fn advise_mmap(mmap: &memmap2::Mmap, header: &GraphFileHeader) {
    use libc::{madvise, MADV_RANDOM, MADV_WILLNEED};

    let base = mmap.as_ptr();
    let node_size = std::mem::size_of::<Node>();
    let edge_size = std::mem::size_of::<Edge>();

    unsafe {
        let nodes_ptr = base.add(header.nodes_offset as usize) as *mut libc::c_void;
        let nodes_len = header.node_count as usize * node_size;

        let edges_ptr = base.add(header.edges_offset as usize) as *mut libc::c_void;
        let edges_len = header.edge_count as usize * edge_size;

        let offsets_ptr = base.add(
            header.nodes_offset as usize + header.node_count as usize * node_size,
        ) as *mut libc::c_void;
        let offsets_len = (header.node_count as usize + 1) * 4;

        // Hot sections: random access during BFS
        madvise(nodes_ptr, nodes_len, MADV_RANDOM);
        madvise(edges_ptr, edges_len, MADV_RANDOM);
        madvise(offsets_ptr, offsets_len, MADV_RANDOM);

        // Prefault hot sections into memory
        madvise(nodes_ptr, nodes_len, MADV_WILLNEED);
        madvise(offsets_ptr, offsets_len, MADV_WILLNEED);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::csr::EdgeInput;
    use crate::graph::edges::EdgeKind;
    use crate::graph::nodes::NodeKind;

    /// Helper: build a test graph with N nodes and E edges.
    fn build_graph(n: u32, edges_spec: &[(u32, u32, EdgeKind)]) -> CsrGraph {
        let mut strings = StringInterner::new();
        let nodes: Vec<Node> = (0..n)
            .map(|i| {
                let name = strings.intern(&format!("node_{}", i));
                Node::new(i, NodeKind::Symbol, name)
            })
            .collect();

        let edge_inputs: Vec<EdgeInput> = edges_spec
            .iter()
            .map(|&(s, t, k)| EdgeInput::new(s, t, k))
            .collect();

        CsrGraph::build(nodes, edge_inputs, strings)
    }

    #[test]
    fn graph_roundtrip() {
        // TEST graph_roundtrip from ARCHITECTURE.md:
        // Build a graph with 100 nodes and 500 edges.
        // Write to disk. Mmap load. Verify every node and edge matches.
        let mut edges_spec = Vec::with_capacity(500);
        for i in 0..500u32 {
            let src = i % 100;
            let tgt = (i * 7 + 13) % 100;
            let kind = EdgeKind::from_u8((i % 11) as u8).unwrap();
            edges_spec.push((src, tgt, kind));
        }

        let graph = build_graph(100, &edges_spec);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.cxgraph");

        write_graph(&graph, &path).unwrap();
        let loaded = load_graph(&path).unwrap();

        // Verify node count and edge count
        assert_eq!(loaded.node_count(), graph.node_count());
        assert_eq!(loaded.edge_count(), graph.edge_count());

        // Verify every node matches
        for i in 0..graph.node_count() {
            let orig = graph.node(i);
            let load = loaded.node(i);
            assert_eq!(orig, load, "node {} mismatch", i);
        }

        // Verify every edge matches
        for i in 0..graph.node_count() {
            let orig_edges = graph.edges_for(i);
            let load_edges = loaded.edges_for(i);
            assert_eq!(
                orig_edges.len(),
                load_edges.len(),
                "edge count mismatch for node {}",
                i
            );
            for (j, (o, l)) in orig_edges.iter().zip(load_edges.iter()).enumerate() {
                assert_eq!(o, l, "edge {},{} mismatch", i, j);
            }
        }

        // Verify reverse edges match
        for i in 0..graph.node_count() {
            let orig_rev = graph.rev_edges_for(i);
            let load_rev = loaded.rev_edges_for(i);
            assert_eq!(orig_rev.len(), load_rev.len());
            for (o, l) in orig_rev.iter().zip(load_rev.iter()) {
                assert_eq!(o, l);
            }
        }

        // Verify strings roundtrip
        for i in 0..graph.node_count() {
            let orig_name = graph.strings.get(graph.node(i).name);
            let load_name = loaded.strings.get(loaded.node(i).name);
            assert_eq!(orig_name, load_name);
        }
    }

    #[test]
    fn file_header_validation() {
        // TEST file_header_validation from ARCHITECTURE.md
        let graph = build_graph(10, &[(0, 1, EdgeKind::Calls), (1, 2, EdgeKind::Calls)]);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.cxgraph");
        write_graph(&graph, &path).unwrap();

        // Valid file loads fine
        assert!(load_graph(&path).is_ok());

        // Corrupt magic bytes
        let mut data = std::fs::read(&path).unwrap();
        data[0] = b'X';
        let bad_magic_path = dir.path().join("bad_magic.cxgraph");
        std::fs::write(&bad_magic_path, &data).unwrap();
        match load_graph(&bad_magic_path) {
            Err(CxError::CorruptGraph(msg)) => assert!(msg.contains("magic")),
            other => panic!("expected CorruptGraph, got {:?}", other),
        }

        // Set version to 999
        let mut data = std::fs::read(&path).unwrap();
        data[4..8].copy_from_slice(&999u32.to_le_bytes());
        // Recompute checksum for the version change
        let bad_version_path = dir.path().join("bad_version.cxgraph");
        // Need to recalculate checksum to pass that check first
        let mut header: GraphFileHeader = unsafe {
            std::ptr::read_unaligned(data.as_ptr() as *const GraphFileHeader)
        };
        header.version = 999;
        header.checksum = header.compute_checksum();
        let header_bytes = unsafe {
            std::slice::from_raw_parts(
                &header as *const GraphFileHeader as *const u8,
                std::mem::size_of::<GraphFileHeader>(),
            )
        };
        data[..64].copy_from_slice(header_bytes);
        std::fs::write(&bad_version_path, &data).unwrap();
        match load_graph(&bad_version_path) {
            Err(CxError::VersionMismatch { found: 999, expected: 2 }) => {}
            other => panic!("expected VersionMismatch, got {:?}", other),
        }

        // Truncate file mid-nodes-section
        let data = std::fs::read(&path).unwrap();
        let truncated = &data[..100]; // way too short
        let truncated_path = dir.path().join("truncated.cxgraph");
        std::fs::write(&truncated_path, truncated).unwrap();
        match load_graph(&truncated_path) {
            Err(CxError::CorruptGraph(_)) => {}
            other => panic!("expected CorruptGraph, got {:?}", other),
        }
    }

    #[test]
    fn file_header_checksum() {
        // TEST file_header_checksum from ARCHITECTURE.md
        // Create valid .cxgraph. Corrupt a header field without updating checksum.
        let graph = build_graph(10, &[(0, 1, EdgeKind::Calls)]);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.cxgraph");
        write_graph(&graph, &path).unwrap();

        // Corrupt node_count field (offset 8) without updating checksum
        let mut data = std::fs::read(&path).unwrap();
        data[8] ^= 0x01;
        let bad_path = dir.path().join("bad_checksum.cxgraph");
        std::fs::write(&bad_path, &data).unwrap();

        match load_graph(&bad_path) {
            Err(CxError::CorruptGraph(msg)) => assert!(msg.contains("checksum")),
            other => panic!("expected CorruptGraph with checksum, got {:?}", other),
        }
    }

    #[test]
    fn mmap_advise_no_crash() {
        // TEST mmap_advise_no_crash from ARCHITECTURE.md
        let graph = build_graph(10, &[(0, 1, EdgeKind::Calls)]);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.cxgraph");
        write_graph(&graph, &path).unwrap();

        // mmap and call advise — should not crash
        let file = std::fs::File::open(&path).unwrap();
        let mmap = unsafe { memmap2::Mmap::map(&file) }.unwrap();

        let header: GraphFileHeader = unsafe {
            std::ptr::read_unaligned(mmap.as_ptr() as *const GraphFileHeader)
        };

        #[cfg(unix)]
        advise_mmap(&mmap, &header);

        // Queries work fine after advise
        let loaded = load_graph(&path).unwrap();
        assert_eq!(loaded.node_count(), graph.node_count());
    }

    #[test]
    fn empty_graph_roundtrip() {
        let graph = build_graph(0, &[]);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.cxgraph");
        write_graph(&graph, &path).unwrap();
        let loaded = load_graph(&path).unwrap();
        assert_eq!(loaded.node_count(), 0);
        assert_eq!(loaded.edge_count(), 0);
    }
}
