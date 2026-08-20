# Sample Similarity Search ("Show Me More Like This")

This ExecPlan is a living document. Maintain it according to `.agent/PLANS.md` whenever implementation changes.

## Purpose / Big Picture

Today a libgooey host can load PCM into a sampler pad (`gooey_engine_sampler_set_slot_buffer` in `src/ffi.rs`) but it has no way to answer the question a musician actually asks: "this kick is nearly right — show me the twelve closest things in my library." A user with tens of thousands of one-shots browses them by filename, which is to say they do not browse them at all.

After this change, a host app holding a library of short samples can ask libgooey two questions and get a ranked answer in a few milliseconds:

Given the audio currently loaded in a pad, which other samples in my library sound most like it? And, given a phrase like "dusty tape-saturated sub kick", which samples match, including samples that were never labelled with any text at all?

The second half of that is the interesting part. Some of these samples were produced by a generative model and the original text prompt was kept. Most were not. The design uses the labelled minority to organize the unlabelled majority, so a text query can reach a sample that has no text attached to it.

The constraint that shapes everything is the deployment target: an Audio Unit v3 (AUv3) app extension on iOS. An "app extension" is a separate process that the host DAW (GarageBand, AUM, Logic) loads into its own address space budget. It is memory-capped far below a normal app, it can be launched and killed at will, and it contains a hard-real-time audio render callback that must never allocate, lock, or touch the filesystem. So the rule this plan follows throughout is: **the extension only ever reads a small precomputed table and multiplies numbers. Every expensive thing — decoding audio, running any machine-learning model, building an index — happens earlier, in the container app, and is written to a shared file.**

The observable validation is `cargo run --example similarity_eval --features native,bounce`. It synthesizes a labelled corpus using libgooey's own kick, snare, hihat2, and tom2 instruments with randomized parameters, indexes it, runs a "more like this" query from each sample, and prints precision@10 — the fraction of the ten returned neighbours that came from the same instrument family as the query. A working implementation prints a figure above 0.90.

## Progress

- [x] (2026-08-20) Author this ExecPlan: survey `src/instruments/sampler.rs`, `src/instruments/sampler_control.rs`, `src/ffi.rs`, `src/filters/`, and `src/visualization/spectrogram.rs`; fix the feature layout, index format, fusion rule, and process split.
- [ ] Milestone 1 — `src/analysis/features.rs`: filterbank descriptor and `SampleFeatures`, with the synthetic-corpus separability test.
- [ ] Milestone 2 — `src/analysis/index.rs`: on-disk index format, brute-force k-NN, FFI surface, `examples/similarity_eval.rs`.
- [ ] Milestone 3 — `src/analysis/text.rs`: tag vocabulary, prompt parsing, score fusion, and the offline audio-to-text projection trainer.
- [ ] Milestone 4 — Result quality: near-duplicate collapse, diversity re-ranking, per-group explanation scores.
- [ ] Milestone 5 — iOS/AUv3 integration: App Group index location, container-app importer, extension search thread, audition path through the existing sampler control queue.
- [ ] Milestone 6 — Scale-out: inverted-file coarse quantizer for libraries beyond roughly 250,000 samples, and incremental import.

## Surprises & Discoveries

- Observation: `rustfft` is an optional dependency enabled only by the `visualization` and `plots` features, and neither is part of the `ios` feature. A Fourier-transform-based descriptor would therefore either not compile for the shipping target or would drag a new dependency into it.
  Evidence: `Cargo.toml` declares `visualization = ["glfw", "gl", "rustfft"]` and `ios = ["bounce"]`; `grep -rn rustfft src/` matches only `src/visualization/spectrogram.rs`.
  Consequence: the descriptor in Milestone 1 is built from a bank of `crate::filters::biquad_bandpass::BiquadBandpass` instances instead. This needs no new dependency, compiles on every feature combination including `ios`, and reuses filters the repository already tests.

- Observation: the cross-thread machinery needed to audition a search result without clicking already exists and is already correct.
  Evidence: `src/instruments/sampler_control.rs` copies PCM on the producer thread, applies replacements only at render-buffer boundaries, and — importantly — drops evicted buffers on a producer thread rather than in the audio callback (`reclaim_from_audio`). `gooey_engine_sampler_queue_slot_buffer` is the public entry point.
  Consequence: Milestone 5 adds no new audio-thread machinery. Auditioning is a call into an existing, tested path.

- Observation: the repository can generate its own labelled evaluation corpus, so this feature can be tested and tuned in CI with no audio fixtures committed.
  Evidence: `crate::bounce::bounce_to_buffer` in `src/bounce.rs` renders an `Engine` offline and is not behind any feature flag — only the WAV writer below it is gated by `bounce` — and `src/instruments/` already contains four acoustically distinct drum families with parameterized configs.
  Consequence: the accuracy target in Validation is measured against synthesized kicks, snares, hats, and toms rather than against a private sample library, which means a contributor can reproduce and improve the descriptor without owning one.

## Decision Log

- Decision: compute the audio descriptor from a bank of bandpass filters rather than from an FFT.
  Rationale: avoids adding `rustfft` to the `ios` feature set, reuses `src/filters/biquad_bandpass.rs`, and gives naturally logarithmic frequency spacing. Short one-shots are only a few thousand samples long, so a 16-band filter sweep is cheaper than windowed transforms anyway.
  Date/Author: 2026-08-20 / Claude

- Decision: keep the index and the search inside the Rust library rather than writing it in Swift.
  Rationale: it becomes testable with `cargo test` on desktop, the tuning harness can run in CI, and the same code serves any future platform. Swift is left with only the two jobs it is uniquely able to do — decoding files with AVFoundation and computing text embeddings with Apple's Natural Language framework.
  Date/Author: 2026-08-20 / Claude

- Decision: brute-force scan by default; add an inverted-file index only past roughly 250,000 rows.
  Rationale: a 64-byte quantized vector scanned across 100,000 rows is 6.4 million multiply-accumulates, a low-single-digit millisecond on any device that runs an AUv3, and it is exact, needs no build step, and never goes stale as the user imports samples. Approximate-nearest-neighbour graph structures cost build time, memory, and correctness for no user-visible gain at this size.
  Date/Author: 2026-08-20 / Claude

- Decision: derive a text vector for unlabelled samples by fitting a linear map from the audio descriptor to the text-embedding space, trained on the subset of samples that do have prompts.
  Rationale: it makes text queries reach the whole library rather than the labelled fraction, it costs one small matrix multiply per sample at import time and nothing at query time, and it requires no neural model in the extension. It is a deliberately modest stand-in for a jointly-trained audio-text model, chosen because it fits the memory budget.
  Date/Author: 2026-08-20 / Claude

## Outcomes & Retrospective

Not yet started. To be written at the completion of each milestone.

## Context and Orientation

A reader new to this repository needs four facts about the current state.

First, **libgooey is a real-time audio synthesis library in Rust**, described in `AGENTS.md`. It builds as a static library, a dynamic library, and a Rust library (`crate-type = ["staticlib", "cdylib", "rlib"]` in `Cargo.toml`). Audio samples are `f32`; time accumulation is `f64`. Its public interface to iOS is a large C foreign-function-interface layer in `src/ffi.rs`, from which `build.rs` generates `include/gooey.h` using cbindgen.

Second, **there is already a sampler**. `src/instruments/sampler.rs` defines `SamplerBuffer`, which owns interleaved PCM as an `Arc<[f32]>` together with a frame count, a channel count of one or two, and a sample rate. It defines `SamplerRack`, a fixed rack of sixteen slots and thirty-two voices. `src/instruments/sampler_control.rs` is the control plane that lets a non-audio thread swap the PCM in a slot safely: the producer copies the data and enqueues it, and the render thread picks it up at a buffer boundary. The host-facing entry point is `gooey_engine_sampler_queue_slot_buffer` in `src/ffi.rs`.

Third, **there is a filter library**. `src/filters/biquad_bandpass.rs` provides `BiquadBandpass::new(sample_rate)`, `set_params(freq, q, gain)`, `process(input) -> f32`, and `reset()`. This plan uses sixteen of them.

Fourth, **there is no analysis code on the shipping path**. `src/visualization/spectrogram.rs` does contain a Fourier analyzer, but it sits behind the `visualization` feature, which pulls in OpenGL and a window toolkit and is not compiled for iOS. Treat it as unavailable.

Some terms used below, defined once here.

An **embedding**, or equivalently a **descriptor** or **feature vector**, is a fixed-length list of numbers that summarizes a sample. Two samples that sound alike should have vectors that are close together. Everything in this plan reduces to producing good vectors and then finding close ones quickly.

**Cosine similarity** between two vectors is their dot product divided by the product of their lengths. It is 1.0 for identical directions and 0.0 for unrelated ones. Because we normalize every stored vector to unit length ahead of time, at query time it is just a dot product.

**Quantization** here means storing each of the 64 numbers as a single signed byte instead of a four-byte float, after scaling. It costs a little precision, which does not matter for ranking, and it makes the table four times smaller and the scan roughly four times faster.

An **App Group** is an iOS mechanism that lets a container app and its app extension share a directory on disk. It is the only sane place to put the index, because the extension cannot see the app's private storage otherwise.

**Memory-mapping** (`mmap`) means asking the operating system to make a file appear as a region of memory without reading it all in. Pages arrive on demand and can be evicted under pressure. This is what allows a memory-capped extension to "hold" a 16 MB index without paying 16 MB of resident memory.

## Plan of Work

The work divides into a signal-processing half and a systems half, and they can be built in that order because the second depends on the first.

### The descriptor

Create `src/analysis/mod.rs`, declared from `src/lib.rs` as `pub mod analysis;` with no feature gate, since it must compile for `ios`.

In `src/analysis/features.rs`, define the analysis entry point:

    pub const EMBEDDING_DIMS: usize = 64;

    #[derive(Clone, Debug)]
    pub struct SampleFeatures {
        pub embedding: [f32; EMBEDDING_DIMS],
        pub duration_seconds: f32,
        pub peak: f32,
        pub rms_db: f32,
        pub class: SampleClass,
    }

    pub fn analyze(
        samples: &[f32],
        frames: usize,
        channels: usize,
        sample_rate: f32,
    ) -> Result<SampleFeatures, String>;

`analyze` validates its arguments the same way `SamplerBuffer::from_interleaved` does — one or two channels, non-zero frames, finite sample rate, `samples.len() == frames * channels`, all values finite — and returns `Err` with a message otherwise, matching the `Result<T, String>` convention stated in `CLAUDE.md`.

Two preprocessing steps happen before any measurement, and both matter more than any individual feature.

The signal is **peak-normalized** to 1.0. Without this, loud samples cluster with loud samples and the search degenerates into a loudness sort. The original peak is retained separately in `SampleFeatures::peak` so a host can still filter or display it.

The **onset** is located, and all time-relative measurements are taken from it rather than from the start of the file. Find it by computing a short-window root-mean-square envelope with a one-millisecond hop, finding the maximum, and walking backwards to the last point below one percent of that maximum. Sample libraries are full of files with tens of milliseconds of leading silence, and without onset alignment two identical kicks trimmed differently land far apart.

The 64 dimensions are then filled in five named groups. Keeping the group boundaries explicit is not cosmetic: Milestone 4 reports per-group scores to explain a match, and Milestone 3 weights the groups differently for different query types.

The **timbre group occupies dimensions 0 through 31**. Instantiate sixteen `BiquadBandpass` filters at logarithmically spaced centre frequencies from 40 Hz to 16 kHz — approximately 40, 60, 90, 135, 200, 300, 450, 670, 1000, 1500, 2200, 3300, 5000, 7500, 11000, 16000 — each with a Q of 1.4, which gives roughly half-octave bandwidths that overlap slightly. Bands whose centre frequency exceeds 45 percent of the sample rate are skipped and their energies left at the floor, so a 22.05 kHz file does not produce garbage in the top bands. Run the mono sum of the normalized signal through all sixteen, rectify each output, and smooth it with a one-pole lowpass at a five-millisecond time constant to get sixteen band envelopes.

Sample those envelopes over four time slices measured from the onset: 0 to 10 ms, 10 to 50 ms, 50 to 200 ms, and 200 ms to the end. These are absolute rather than proportional windows, deliberately — for percussive one-shots the absolute timing of the transient is what the ear keys on, and a proportional split would make a 100 ms hat and a 2 s crash look alike. Take the mean of each band envelope within each slice, convert to decibels with a floor at -80 dB, and apply a discrete cosine transform across the sixteen bands. Discard the zeroth coefficient, which is just the overall level in that slice and duplicates information we deliberately normalized away, and keep the next eight. Eight coefficients times four slices is 32 dimensions. This is a filterbank cepstrum: the low coefficients describe broad spectral tilt, the higher ones describe finer structure such as the metallic ringing that distinguishes a hi-hat from a noise burst.

The **envelope group occupies dimensions 32 through 43**. Store the base-ten logarithm of the attack time, defined as the interval from onset to peak; the logarithm of the effective duration, defined as the time for the running root-mean-square to fall 40 dB below its peak, or the file length if it never does; the crest factor, being peak divided by overall root-mean-square; the slope of a straight line fitted to the decibel envelope over the decay portion; and an eight-point curve, the overall envelope in decibels sampled at eight logarithmically spaced times after the onset and then normalized so its first value is zero. Logarithmic spacing puts most of the resolution in the first fifty milliseconds where percussive samples actually differ.

The **spectral-motion group occupies dimensions 44 through 51**. From the band energies already computed, derive the spectral centroid in each of the four time slices, expressed as a log-frequency value and scaled to roughly unit range. Add the difference between the last and first centroid, which captures the extremely characteristic darkening of a decaying drum. Add spectral flatness — the ratio of the geometric to the arithmetic mean of the band energies, near 1.0 for noise and near 0.0 for a pure tone — averaged over the sample and again as a first-to-last difference. Add the ratio of energy above 2 kHz to energy below it, again as a value and a change.

The **tonality group occupies dimensions 52 through 59**. Estimate a fundamental frequency using normalized autocorrelation in the time domain over a window centred 30 ms after the onset, searching lags corresponding to 40 Hz through 1200 Hz. A time-domain estimator is chosen over a frequency-domain one for the same dependency reason as everything else, and it is well suited to the short windows involved. Store the logarithm of the estimated fundamental; the height of the autocorrelation peak, which serves as a confidence and is near zero for unpitched material; the sine and the cosine of the pitch class angle, where pitch class is the fundamental expressed in semitones modulo twelve and the angle is that value times thirty degrees. Sine and cosine are used rather than the raw pitch class so that B and C come out adjacent instead of maximally distant. Add a harmonic-to-noise ratio, an inharmonicity measure, and two coefficients describing the relative strength of even and odd harmonics. Every value in this group is multiplied by the pitch confidence before being stored, so that unpitched samples contribute nothing here rather than contributing noise.

The **stereo group occupies dimensions 60 through 63**. Store the ratio of side energy to mid energy, the inter-channel correlation, the change in that width between the first and second half, and a flag-like value for true mono. For a mono input these are set to a fixed neutral value.

Finally, define `SampleClass` as a small enum — `PercussiveTransient`, `Sustained`, `Tonal`, `Noise`, `Loop` — assigned by thresholds on attack time, effective duration, and pitch confidence. It exists to let the search cheaply restrict itself to plausible candidates before scoring anything.

The raw 64 numbers are not comparable to each other; a log-duration ranges over units while a cepstral coefficient ranges over tens. Two normalizations fix this. Each dimension is standardized by subtracting a mean and dividing by a standard deviation computed across the whole library — these 128 numbers live in the index header and are recomputed when the library grows substantially. Then the whole vector is scaled to unit length so that cosine similarity is a plain dot product. Only after both steps is the vector quantized to signed bytes, by clamping to plus or minus four standard deviations and scaling to the range -127 to 127.

### The index

In `src/analysis/index.rs`, define the on-disk format. It is a single file, laid out so it can be memory-mapped and read directly with no parsing pass.

A fixed header holds a magic number, a format version, the row count, the embedding dimensionality, the 64 per-dimension means and 64 standard deviations, the text-embedding dimensionality where present, an optional projection matrix, and byte offsets to each subsequent section.

A dense **audio vector block** follows: `row_count * 64` signed bytes, row-major. This is the only region the scan touches, which is the point — 100,000 samples is 6.4 MB of contiguous bytes, and a linear pass over it is close to the best case a memory system can be given.

A dense **text vector block**, of the same shape, holds either the embedding of the sample's real prompt or, for unlabelled samples, the projection of its audio vector. A per-row flag distinguishes the two, because a real prompt should be trusted more than an inferred one.

A **metadata block** holds one fixed-size record per row: a stable 64-bit sample identifier, the class enum, duration in milliseconds, the original peak and root-mean-square in quantized decibels, a 128-bit tag bitset, and offsets into the string blob. Approximately 32 bytes.

A **string blob** holds file paths, security-scoped bookmark data, display names, and prompt text, all length-prefixed. It is deliberately last and is never touched during a scan; only the handful of rows that are actually returned resolve their strings.

At roughly 160 bytes per row all told, a 100,000-sample library is a 16 MB file, of which the search hot path touches 6.4 MB.

Define the query API:

    pub struct SampleIndex { /* mmap or owned bytes */ }

    pub struct QueryFilter {
        pub class: Option<SampleClass>,
        pub max_duration_ms: Option<u32>,
        pub require_tags: u128,
        pub exclude_ids: &[u64],
    }

    pub struct Match {
        pub id: u64,
        pub score: f32,
        pub audio_score: f32,
        pub text_score: f32,
        pub group_scores: [f32; 5],
    }

    impl SampleIndex {
        pub fn open(path: &Path) -> Result<Self, String>;
        pub fn from_bytes(bytes: &[u8]) -> Result<Self, String>;
        pub fn len(&self) -> usize;
        pub fn search(&self, query: &Query, k: usize, filter: &QueryFilter) -> Vec<Match>;
    }

`SampleIndex` must be `Send + Sync`, which it is naturally when it holds a read-only mapping, so the extension can hold one and search from any thread.

`search` performs a single linear pass. For each row it first checks the filter, which is a few integer comparisons and rejects most rows in a typical query without touching the vector at all; then it accumulates the dot product of the query vector against the row's 64 bytes; then it inserts into a bounded max-heap of size k. Write the accumulation loop over `i8` values into an `i32` accumulator so that the compiler can vectorize it, and add a `#[cfg(target_arch = "aarch64")]` NEON path only if profiling on device shows the scalar loop is not fast enough — it very likely will be.

### Text, tags, and the bridge between them

`src/analysis/text.rs` handles the prompt side.

Prompts are first reduced to a **tag set**. Define a controlled vocabulary of roughly 128 audio-relevant terms covering instrument families (kick, snare, clap, hat, tom, ride, bass, pad, lead, vocal, foley), production adjectives (lofi, tape, vinyl, saturated, distorted, clean, warm, bright, dark, dusty, crunchy), era and genre markers (808, 909, boom-bap, jungle, trap, house), and articulation (short, long, punchy, soft, ghost, roll). Parsing is deliberately simple: lowercase, strip punctuation, match unigrams and a small list of bigrams against the vocabulary including a hand-written synonym table, and set the corresponding bits in the 128-bit bitset. Each tag gets one bit, so the whole tag set is two 64-bit words and overlap scoring is two `count_ones` calls. This layer requires no model, works the moment a prompt exists, and is what the interface shows the user as the visible reason a result matched.

Prompts are also, where the platform allows, reduced to a **dense text vector**. This is the one step the Rust library does not perform. On iOS the container app calls `NLEmbedding.sentenceEmbedding(for: .english)` from Apple's Natural Language framework, which is on-device, free, and requires shipping no model; it hands the resulting vector back across the FFI as a float array to be stored in the row. If no text embedder is available on a given platform, the text vector block is simply absent and the system degrades to tags alone. Nothing in the extension ever runs the embedder — it only reads vectors that were written at import time.

The **projection** ties the two spaces together and is what lets a text query find an unlabelled sample. Let `X` be the matrix of audio vectors for the subset of samples that do have real prompts, and `Y` the matrix of their text vectors. Fit a linear map by ridge regression, solving `W = (XᵀX + λI)⁻¹ XᵀY`. Because the audio space is only 64-dimensional, `XᵀX` is a 64 by 64 matrix and the solve is a Cholesky decomposition that takes microseconds and needs no linear-algebra dependency — write it directly. Choose λ by holding out twenty percent of the pairs and picking the value that maximizes the cosine between predicted and actual text vectors on the held-out set.

Apply `W` to every unlabelled sample's audio vector to give it an inferred text vector. Two guards apply. Do not fit the projection at all until there are at least 500 prompt-bearing samples, since below that it will overfit and produce confident nonsense; until then, unlabelled samples simply have no text vector and are unreachable by text query, which is honest. And keep the per-row flag distinguishing real from inferred vectors so that ranking can discount inferred ones by a fixed factor, around 0.7, and so the interface can avoid claiming a sample matches words that were never associated with it.

The trainer is a desktop-only binary, `examples/train_projection.rs`, which reads an index, fits `W`, reports held-out quality, and writes the matrix back into the index header. It never runs on iOS.

### Fusing the scores

A query carries an optional audio vector, an optional text vector, and an optional tag set. The combined score is

    score = w_a * cos(audio_q, audio_c)
          + w_t * cos(text_q, text_c) * trust_c
          + w_g * tag_overlap(tags_q, tags_c)

where `trust_c` is 1.0 for a real prompt and 0.7 for an inferred one, and `tag_overlap` is the Jaccard index of the two bitsets — intersection popcount over union popcount.

The weights depend on what the query actually has, and are renormalized to sum to one after zeroing the unavailable terms. For "more like this" started from audio, use 0.75, 0.15, 0.10. For a typed text query, use 0.20, 0.60, 0.20 — audio still contributes because the seed pad's sound is context even when the user is typing. For a pure text search with no pad loaded, use 0.0, 0.75, 0.25.

Within the audio term, the five feature groups are weighted rather than treated equally: timbre 0.40, envelope 0.30, spectral motion 0.15, tonality 0.10, stereo 0.05. Envelope is weighted heavily because for one-shots it carries most of the perceptual identity. Fold these weights into the stored vectors at index-build time by scaling each group, so that the query-time inner loop stays a flat dot product with no per-dimension multiply.

### Making the results feel right

Raw nearest-neighbour output is a poor interface, for two specific reasons that are worth fixing explicitly.

The first is **near-duplicates**. Real libraries contain the same sample many times over, across packs, at different bit depths, with different amounts of leading silence. A top-twelve list that is nine copies of one sound is useless. After scoring, collapse any group of results whose mutual cosine similarity in the timbre-plus-envelope subspace exceeds 0.985 and whose durations agree within five percent, keeping the highest-scoring member and attaching a count. Search for `k * 4` candidates so that there are enough left after collapsing.

The second is **redundancy**. Even after removing duplicates, the twelve nearest neighbours are often twelve minor variations. Re-rank the surviving candidates greedily by maximal marginal relevance: repeatedly pick the candidate maximizing `0.7 * score(query, c) - 0.3 * max_similarity(c, already_selected)`. The result set covers the neighbourhood rather than piling up in one corner of it.

Both of these operate on at most a few dozen candidates, so their cost is irrelevant next to the scan.

Populate `Match::group_scores` so the interface can say something concrete — "similar decay, brighter" comes from comparing the envelope and timbre sub-scores — rather than showing an uninterpretable percentage.

Finally, log which result the user actually auditions and which they load. That signal, accumulated, allows fitting a diagonal weighting over the 64 dimensions from triplets of the form "given query q, the user chose a over b". Because the learned object is diagonal, applying it stays a scaling of the stored vectors and the query-time cost remains a dot product. This is worth doing only after the rest works and there is real usage data; it is listed last in Milestone 4 for that reason.

### Living inside an AUv3

The process split is the whole game here.

The **container app** owns writes. It walks the user's imported files, decodes each with AVFoundation, calls `gooey_analyze_sample` for the descriptor, calls `NLEmbedding` for the prompt vector where a prompt exists, and appends a row. It stores a security-scoped bookmark rather than a path, because an extension cannot reopen an arbitrary user file from a path alone. It writes the index into the App Group container directory, and it writes atomically — build into a temporary file and rename — so that an extension mapping the file concurrently never observes a half-written state. Periodically it recomputes the normalization statistics and refits the projection, which requires rewriting the vector blocks; do this as a background compaction, not on every import.

The **extension** only reads. On instantiation it maps the index, which costs essentially nothing and no meaningful resident memory. It runs searches on a background dispatch queue and delivers results to the interface thread. It decodes audio only for the specific sample the user asks to hear — never for the library — and it caps that decode, refusing to load anything longer than a few seconds into a pad. It never rebuilds the index; if the file version does not match what the extension understands, it reports that the library needs to be reopened in the main app rather than attempting a migration inside a memory-capped process.

The **audio thread** is not involved in any of this, and adding search must not change that. When the user chooses to audition a result, the decoded PCM goes through `gooey_engine_sampler_queue_slot_buffer`, which already copies on the calling thread, hands over at a render-buffer boundary, and disposes of the evicted buffer on a producer thread. That path is tested and this feature should use it unchanged.

Three iOS-specific hazards are worth stating plainly. Extension memory limits are far tighter than app limits and are not documented as a stable number, so the index must be mapped rather than read and library audio must never be held resident — the design above satisfies both. Extensions are terminated aggressively, so all state that matters lives in the shared file and the extension must tolerate being restarted mid-browse. And an AUv3 may be instantiated several times inside one host; the mapping is read-only, so the instances share physical pages, which is another reason to avoid loading the index into private memory.

### The FFI surface

Add to `src/ffi.rs`, following the conventions already in force there — `#[no_mangle] extern "C"`, null checks on every pointer, `/// # Safety` on every unsafe function, out-parameters for arrays as in `gooey_engine_get_channel_peaks`:

    gooey_analyze_sample(samples, frames, channels, sample_rate, out_embedding, out_meta) -> bool
    gooey_index_open(path) -> *mut GooeyIndex
    gooey_index_free(index)
    gooey_index_row_count(index) -> u32
    gooey_index_search(index, query, k, out_ids, out_scores) -> u32
    gooey_index_match_detail(index, id, out_group_scores, out_tag_bits) -> bool
    gooey_index_row_string(index, id, field, out_buf, buf_len) -> u32

The query is passed as a small repr(C) struct holding the optional vectors, the tag bitset, the filter, and the weight preset, so that the signature does not grow to a dozen arguments. `GooeyIndex` is allocated with `Box::into_raw` and released with `Box::from_raw`, matching the engine handle.

## Concrete Steps

Work from the repository root, `/home/user/libgooey`.

Milestone 1 adds `src/analysis/mod.rs` and `src/analysis/features.rs`, declares `pub mod analysis;` in `src/lib.rs`, and adds `tests/analysis_features.rs`. That test does something this repository is unusually well positioned to do: it generates its own labelled corpus. Using the existing `KickDrum` (`src/instruments/kick.rs`), `SnareDrum` (`src/instruments/snare.rs`), `HiHat2` (`src/instruments/hihat2.rs`), and `Tom2` (`src/instruments/tom2.rs`) instruments together with `crate::bounce::bounce_to_buffer(&mut Engine, BounceLength::Samples(n))`, render 50 variants of each with randomized but family-consistent parameters, seeded deterministically so the test is reproducible. Analyze all 200, and assert that for each sample the ten nearest by cosine similarity are at least 90 percent from the same family. This requires no audio fixtures in the repository, runs in CI, and directly measures the only thing that matters about the descriptor.

Note two things about this corpus. `bounce_to_buffer` is not behind any feature flag — only the WAV writer `bounce_to_wav` is gated by `bounce` — so the test needs no feature arguments. And it returns mono audio, so the four stereo dimensions will be constant across the whole synthetic corpus; that is expected and harmless, but do not read their zero contribution as a bug.

    cargo test --test analysis_features --verbose

Expect output resembling:

    running 3 tests
    test analyze_rejects_malformed_input ... ok
    test embedding_is_unit_length_and_finite ... ok
    test synthetic_corpus_precision_at_10 ... ok
    precision@10 = 0.96 (kick 0.98, snare 0.94, hihat 0.97, tom 0.95)

Milestone 2 adds `src/analysis/index.rs`, the FFI functions, and `examples/similarity_eval.rs`. `Cargo.toml` sets `autoexamples = false`, so the example must be registered explicitly with an `[[example]]` block; it needs no `required-features`, because it only renders offline and prints, opening no audio device. The example builds the same synthetic corpus, writes an index to a temporary path, reopens it, queries, and prints precision@10 plus a timing figure.

    cargo run --example similarity_eval

Expect:

    indexed 200 samples in 41 ms (12.8 KB audio vectors)
    mean query time over 200 queries: 0.09 ms
    precision@10 = 0.96

Then verify the scan holds up at realistic scale by synthesizing 100,000 random rows and timing a query; this belongs in the example behind a `--bench` flag. A query should complete in single-digit milliseconds on a desktop and should be re-measured on device before Milestone 6 is considered necessary at all.

Milestone 3 adds `src/analysis/text.rs` and `examples/train_projection.rs`. Test the tag parser against a table of real prompt strings and expected tag sets. Test the ridge solve against a synthetic case where the true linear relationship is known, asserting recovery within tolerance.

Milestones 4 through 6 are additive to the above and each carry their own tests: duplicate collapse against a corpus seeded with deliberate near-duplicates, diversity re-ranking asserting that the returned set spans more of the space than the raw top-k, and the coarse quantizer asserting that its top-10 agrees with the brute-force top-10 on at least 95 percent of queries.

After every milestone, run the full validation sequence from `CLAUDE.md`:

    cargo build
    cargo build --example kick --features native,crossterm
    cargo test --verbose
    cargo fmt --all -- --check
    cargo clippy --all-targets --all-features

Additionally confirm the iOS feature set still compiles, since the whole descriptor design exists to keep this true:

    cargo build --features ios --no-default-features

## Validation and Acceptance

The system is working when the following are all true.

`cargo test --test analysis_features` passes and reports precision@10 above 0.90 on the synthetic four-family corpus. This test fails before Milestone 1 because the module does not exist, and passes after.

`cargo run --example similarity_eval` prints a mean query time under 1 ms for the 200-sample corpus and under 10 ms for the 100,000-row synthetic benchmark.

`cargo build --features ios --no-default-features` succeeds, proving the analysis and search path carries no desktop-only dependency.

A text query for a term in the vocabulary, run against an index in which the projection has been fitted, returns samples that have no prompt of their own but belong to the right family — this is the specific behaviour that justifies the projection, and it should be asserted directly in a test that labels only half a synthetic corpus and queries for the other half.

Requesting twelve results from a corpus containing deliberate near-duplicates returns twelve audibly distinct samples, not four sounds repeated.

On device, opening the AUv3, tapping a loaded pad, and choosing "more like this" returns results without any audible glitch in the render stream, and auditioning a result swaps the pad's audio without a click. The absence of a click is the acceptance criterion for having used the existing control queue correctly rather than writing a new path.

## Idempotence and Recovery

Every step is additive; no existing file is rewritten in a way that changes current behaviour, and the analysis module is inert unless called. Re-running any test or example is safe. The index writer builds into a temporary file and renames, so an interrupted import leaves the previous index intact and a partially-written temporary file that can be deleted. Index format changes are handled by the version field in the header: a reader that does not recognize the version refuses to open the file and reports that the library must be reopened in the container app, rather than attempting an in-place migration inside the memory-capped extension. Deleting the index file is always a safe recovery action, costing only a reimport.

## Interfaces and Dependencies

No new crate dependencies. This is a deliberate constraint and the reason for several of the decisions above; if it is ever relaxed, revisit the filterbank descriptor in favour of a Fourier one and the hand-written Cholesky solve in favour of a library.

In `src/analysis/features.rs`, define `EMBEDDING_DIMS`, `SampleClass`, `SampleFeatures`, and `analyze` as given in the Plan of Work.

In `src/analysis/index.rs`, define `SampleIndex`, `IndexBuilder`, `Query`, `QueryFilter`, and `Match` as given, with `SampleIndex: Send + Sync`.

In `src/analysis/text.rs`, define:

    pub struct TagSet(pub u128);
    pub fn parse_prompt(prompt: &str) -> TagSet;
    pub fn tag_overlap(a: TagSet, b: TagSet) -> f32;
    pub fn fit_projection(audio: &[[f32; EMBEDDING_DIMS]], text: &[Vec<f32>], lambda: f32)
        -> Result<Projection, String>;

Existing code this depends on: `crate::filters::biquad_bandpass::BiquadBandpass` for the filterbank; `crate::instruments::sampler::SamplerBuffer` for the shape of PCM validation, which `analyze` should mirror; `crate::bounce::bounce_to_buffer` for offline rendering in tests, which is not feature-gated and returns mono samples.

Platform code outside this repository, for reference by whoever writes the Swift side: `AVAudioFile` for decoding, `NLEmbedding.sentenceEmbedding(for:)` for prompt vectors, `FileManager.containerURL(forSecurityApplicationGroupIdentifier:)` for the shared directory, and security-scoped bookmarks via `URL.bookmarkData(options:.withSecurityScope)` so the extension can reopen user files.
