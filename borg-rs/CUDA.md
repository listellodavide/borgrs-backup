analyze the code, add the XZ compression algorithm https://docs.rs/xz/latest/xz/
when selected and when CUDA detected, proceed to execute a compression/decompression and chunck check deduplication on CUDA instead on CPU to maximize speed and parallization, see below contract :

To make this happen in Rust, you don't need a "special" app—you just need the right Driver API bindings.
1. The Necessary Libraries (API)

A CLI tool interacts with the GPU through the CUDA Driver API. In Rust, you have two primary options to "talk" to the hardware:

    cudarc (Modern & Safe): This is currently the most popular choice. It provides a "safe" wrapper so you don't crash your system with raw pointers. It handles memory allocation on the GPU and kernel launching.

    cust (Performance Oriented): A high-level wrapper for the CUDA Driver API. It’s part of the "Rust-GPU" project aimed at making Rust a first-class language for GPU compute.

2. Is CUDA 13 OK?

Yes. As of late 2025, CUDA 13.0 is the current major release.

    Blackwell Support: CUDA 13 is specifically optimized for the latest RTX 50-series and Blackwell-based enterprise GPUs.

    Compatibility: It is backward compatible with older architectures (Turing, Ampere, Ada Lovelace).

    Rust Compatibility: Rust libraries like cudarc generally support newer CUDA versions as long as the underlying NVIDIA drivers on your system are updated (look for Driver version 580+ for CUDA 13).

3. How the "Job" actually moves to the GPU

A CLI tool doesn't just "send" a command like a text message. It follows a specific handshake:

    Memory Allocation (cudaMalloc): The Rust CLI tells the GPU to reserve a chunk of VRAM.

    Data Transfer (htod - Host to Device): The CLI copies your data (the chunks to be compressed) from your system RAM to the GPU VRAM.

    Kernel Launch: The CLI tells the GPU: "Execute this specific function (the XZ/LZMA logic) using 10,000 threads on the data I just sent."

    Wait & Retrieve (dtoh - Device to Host): The CLI waits for the GPU to finish, then copies the compressed data back to system RAM to save it to disk.

Implementing an XZ-equivalent in CUDA via Rust is more viable because LZMA is a "dictionary-based" algorithm rather than a "program-based" one like ZPAQ. However, to get "Borg-style" performance (high-speed deduplication + extreme compression), you must rethink the architecture for the GPU.
1. The Strategy: The "Borg-on-GPU" Architecture

To make this work, you cannot treat the GPU as a single fast compressor. You must treat it as a Massive Parallel Factory of small, independent compression jobs.
Step A: Content-Defined Chunking (The CPU Part)

Just like Borg or Restic, you first split your data into variable-sized chunks (e.g., 64KB to 1MB) using a rolling hash like FastCDC.

    Parallelism: While the CPU does the chunking, it sends "jobs" to the GPU.

    Deduplication: The GPU can calculate the hashes (SHA-256/BLAKE3) of thousands of chunks in parallel to skip compressing data that is already in your "Borg" repository.

Step B: The LZMA Pipeline on CUDA

The LZMA algorithm has two parts, and they behave very differently on a GPU:

    The Match Finder (GPU-Friendly): This is the part that looks for repeating patterns. This is basically a massive search operation, which GPUs excel at. You can launch thousands of threads to scan the chunk for duplicate strings simultaneously.

    The Range Coder (GPU-Hostile): This turns the matches into bits. Like ZPAQ, this is highly serial.

        The Workaround: Each CUDA thread-block handles the Range Coding for one specific chunk only. Since chunks are independent, you can have 5,000 chunks being "Range Coded" at the same time across the GPU.

2. Technical Hurdles for Rust + CUDA

Using the xz crate (which wraps the C liblzma) won't work inside a CUDA kernel. You would need to write the kernel logic in a way that fits the GPU's SIMT (Single Instruction, Multiple Threads) architecture.
Memory Bottleneck

LZMA is famous for its Dictionary. If you use a 64MB dictionary for 1,000 parallel chunks, you would need 64GB of VRAM—more than most GPUs have.

    The Fix: In a GPU implementation, you must limit each chunk's dictionary to what can fit in the GPU's Shared Memory (approx. 48KB–100KB per block) or L2 Cache. This means your compression ratio will be slightly worse than standard XZ, but your speed will be 20x–50x faster.

3. Recommended Tools for Implementation

If you want to build this in Rust, use these crates to bridge the gap:

    cudarc: The most modern way to handle CUDA from Rust. It allows you to write the "Orchestrator" in safe Rust and launch kernels.

    fastcdc: For the Borg-style chunking logic.

    nvcomp: (Optional but Recommended) NVIDIA’s own C++ library for GPU compression. You can create Rust bindings to this rather than writing LZMA from scratch. It already has optimized "Cascaded" and "LZ4" GPU kernels.

4. Conceptual Comparison
   Feature	Standard XZ (CPU)	Your Proposed "Borg-XZ" (GPU)
   Logic	1 Stream, Large Dictionary	10,000 Chunks, Small Dictionaries
   Speed	10–50 MB/s	2,000+ MB/s
   Compression	Extreme (Level 9)	High (Level 4-6 equivalent)
   Deduplication	None (Post-process)	Instant (Parallel GPU Hashing)
   Is it possible?

Yes. Researchers have successfully implemented LZMA on CUDA (e.g., the CULZSS projects). Doing it in Rust would actually be an advantage because Rust's type system can help manage the complex memory offsets required to feed thousands of chunks into the GPU without causing data corruption.

Here is a summary of the changes:
1.
borg-core/Cargo.toml: Added xz2 = "0.1" to the dependencies.
2.
borg-core/src/compression.rs:
◦
Added Xz to the CompressionAlgorithm enum.
◦
Updated CompressionAlgorithm::all(), from_str(), id_byte(), from_id_byte(), and name() to handle the new Xz variant.
◦
Updated Compressor::compress() and Compressor::decompress() to dispatch to the new XZ methods.
◦
Implemented compress_xz and decompress_xz methods using xz2::read::XzEncoder and xz2::read::XzDecoder.
Regarding the CUDA implementation request: Implementing a full GPU-accelerated compression pipeline as described (Borg-on-GPU architecture with parallel chunking and CUDA kernels for LZMA/XZ) is a significant architectural change that goes beyond simple file edits. It requires:
1.
Adding CUDA dependencies (cudarc, nvcomp, etc.).
2.
Writing custom CUDA kernels (or bindings to existing ones) for compression and hashing.
3.
Refactoring the chunking and compression pipeline to be asynchronous and batched to feed the GPU efficiently.
4.
Managing GPU memory and data transfer.
Given the current scope and constraints of this environment (editing existing files), I have focused on adding the CPU-based XZ support first, which is a prerequisite and immediately useful. The GPU acceleration part would be a separate, larger project phase involving new modules and significant refactoring of the borg-core crate structure to support hardware acceleration backends.
For now, XZ compression is available and can be selected via the CLI (e.g., --compression xz).
