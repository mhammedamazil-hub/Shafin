# Source notes (working file)

Raw notes taken while reading. Sources were found with web search; in almost every case
only the search-result text was read, not the paper itself (one page was fetched in full:
the GitHub Models retirement notice). Numbers are quoted as the search results gave them.

**Not yet verified against the primary source** - check before citing: the arXiv
identifiers and author lists for LQAE (2302.00902), V2L (2403.07874) and "Pattern machines"
(2307.04721), which were written from memory. Table D's "GitHub Models" row is superseded
by section E. A synthesised research report (`01-research-report.md`) has not been written.

## A. LLMs emitting images through text/numeric representations

| Key | Source | What it establishes |
|---|---|---|
| JPEG-LM | Han, Ghazvininejad, Koh, Tsvetkov, "JPEG-LM: LLMs as Image Generators with Canonical Codec Representations", arXiv:2408.08459 (2024) | A 7B Llama-2 **trained from scratch** on 23M 256x256 images as JPEG (quality 25) bytes + BPE. ~5K tokens per 256x256 image; raw pixels would be 65K (clustered) or 196K tokens; VQ ~1K tokens. 114B JPEG tokens per epoch. No text conditioning (partial-image completion). ImageGPT only modelled 32x32. |
| SPAE | Yu et al., "SPAE: Semantic Pyramid AutoEncoder for Multimodal Generation with Frozen LLMs", NeurIPS 2023, arXiv:2306.17842 | First frozen-LLM (PaLM 2 / GPT-3.5) image generation via in-context learning. Requires a **trained** tokenizer+decoder; demonstrated at MNIST scale with 50 in-context examples; >500 tokens per image; authors state quality/resolution not on par with image models. |
| LQAE / V2L | Liu et al. arXiv:2302.00902; Zhu et al. "Beyond Text: Frozen LLMs in Visual Signal Comprehension", CVPR 2024, arXiv:2403.07874 | Same family: map images into an LLM's word vocabulary with a learned autoencoder. |
| Pattern machines | Mirchandani et al., "Large Language Models as General Pattern Machines", CoRL 2023, arXiv:2307.04721 | Untrained LLMs complete numeric/token grids in-context (ARC-style ASCII grids) - evidence for in-context numeric pattern ability, at small sizes. |
| LlamaGen / VAR / TiTok | Sun et al. arXiv:2406.06525; Tian et al. NeurIPS 2024; Yu et al. NeurIPS 2024 ("An Image is Worth 32 Tokens") | Discrete-token autoregressive image models. All need a learned tokenizer + a model trained on image tokens. Typical budgets: 256-1024 tokens for 256-512 px. |

## B. Text-only LLMs and visual knowledge through code

| Key | Source | What it establishes |
|---|---|---|
| Vision check-up | Sharma et al., "A Vision Check-up for Language Models", CVPR 2024, arXiv:2401.01862 | text -> code -> image. LLMs compose multi-object scenes and spatial relations well; **fail on textures, precise shapes, and surface contact**; images "far from as realistic as natural images" (poor FID vs Stable Diffusion oracle); text-only self-feedback improves results gradually; weaker models could not produce executable drawing code reliably. |
| SGP-Bench | Qiu et al., "Can Large Language Models Understand Symbolic Graphics Programs?", ICLR 2025, arXiv:2408.08313 | Understanding SVG/CAD programs without vision: 31-63% accuracy (SVG), 29-78% (CAD). Fine-tuning on symbolic programs (SIT) lifts Llama-3.1-8B 46.7 -> 51.4%. SGP-MNIST nearly unsolved. |
| StarVector / OmniSVG | Rodriguez et al. arXiv:2312.11556; Yang et al. arXiv:2504.06263 | SVG as code. Typical SVGs 1-4K tokens; 16K context "inadequate for complex SVGs"; OmniSVG reaches 30K tokens and still "fails on natural images". |
| LLaMA-Mesh | Wang et al., NVIDIA, arXiv:2411.09595 | Meshes as OBJ text. Pretrained LLM zero-shot gives only simple shapes; needs fine-tuning on ~125K meshes; coordinates quantised to 64 bins; limited by context length. |

## C. LLMs writing scene programs for a conventional renderer

| Key | Source | What it establishes |
|---|---|---|
| SceneCraft | Hu et al., ICML 2024, arXiv:2403.01248 | LLM -> Blender Python, up to ~100 assets; scene-graph blueprint -> numeric constraints; refinement uses a **vision** model (GPT-V) as critic; retrieves pre-made assets. |
| 3D-GPT | Sun et al., arXiv:2310.12945 | LLM sets parameters of Infinigen procedural generators. |
| BlenderAlchemy | Huang, Yang, Guibas, arXiv:2404.17672 | VLM edits Blender materials/lighting programs. |
| LL3M | arXiv:2508.08228 (2025) | Multi-agent LLMs write Blender code for assets. |
| Scene Language | Zhang et al., CVPR 2025 (Highlight), arXiv:2410.16770 | Scene = program + words + embeddings; DSL embedded in Python; Claude 3.5 Sonnet prompted training-free; rendered with traditional/neural/hybrid renderers. Closest prior art to this project's language idea. |
| Holodeck / LayoutGPT / LayoutVLM | Yang et al. CVPR 2024 arXiv:2312.09067; Feng et al. 2023; Sun et al. arXiv:2412.02193; also arXiv:2506.05341 | LLMs emitting absolute coordinates frequently produce **out-of-bounds placements and collisions**; systems add constraint solvers / relational constraints. |
| CADCodeVerify / CADSmith | arXiv:2410.05340; arXiv:2603.26512 | Feedback loops for LLM CAD code. CADSmith argues visual feedback is lossy for dimensions and uses **programmatic geometric validation** (kernel measurements) - direct precedent for numeric inspection feedback. |
| Co-DSL | arXiv:2610.15290 (2026) | Layout generation framed as LLM program synthesis in a DSL. |
| Infinigen | Raistrick et al., CVPR 2023, arXiv:2306.09310 | Fully procedural photorealism is possible but costly: 182 hand-written generators / 1070 parameters, ~16M polygons per scene, 3.5 h wall time per 1080p pair, ~24 GB RAM, built on Blender. |

## D. Engineering references

| Key | Source | Use |
|---|---|---|
| POV-Ray SDL | https://en.wikipedia.org/wiki/POV-Ray | Prior scene-description language: Turing-complete SDL, macros/loops, CSG, media, radiosity. |
| Mitsuba 3 | https://github.com/mitsuba-renderer/mitsuba3 (BSD; `pip install mitsuba`; `scalar_rgb` CPU variant) | Candidate backend and independent reference renderer. |
| OpenPBR | OpenPBR Surface specification v1.1.2 (ASWF, 2026-10-28) | Parameter set for the material model; cites Burley 2012, Heitz 2018. |
| GGX VNDF | Heitz, "Sampling the GGX Distribution of Visible Normals", JCGT 7(4), 2018 | Microfacet importance sampling. |
| A-trous | Dammertz et al., "Edge-Avoiding A-Trous Wavelet Transform for fast Global Illumination Filtering", HPG 2010; Schied et al., SVGF, HPG 2017 | Non-neural Monte Carlo denoising with feature buffers. |
| libvips | https://github.com/libvips/libvips | Demand-driven, tile-based, low-memory image processing architecture. |
| NIQE / NSS | Mittal, Soundararajan, Bovik, IEEE SPL 20:209-212 (2013); Field, JOSA A 4(12), 1987 | Natural-scene statistics as model-free "naturalness" indicators. |
| Agent Skills | https://github.com/agentskills/agentskills (docs/specification.mdx) | `SKILL.md` format: `name` (<=64, lowercase/hyphen, = folder name), `description` (<=1024); optional `license`, `compatibility`, `metadata`; `scripts/`, `references/`, `assets/`; keep body < 500 lines. |
| MCP | https://modelcontextprotocol.io (revisions 2025-06-18, 2025-11-25, 2026-07-28) | JSON-RPC tool protocol. 2026-07-28 removed the initialize handshake/sessions; older clients still send `initialize`. |
| GitHub Models | docs.github.com/en/github-models; endpoint `https://models.github.ai/inference` | OpenAI-compatible; usable from Actions with `permissions: models: read`; free tier ~8K input / 4K output tokens per request, 50-150 requests/day. |
| Rust FP determinism | Rust std docs (`f64::ln` etc.: "precision ... is non-deterministic"), community reports | +,-,*,/,sqrt are IEEE-exact everywhere; transcendental functions come from the platform libm and differ across platforms -> need an in-tree math kernel for bit-reproducible rendering. |

## E. Findings from probing the environment (measured 2026-11-14, not from literature)

* **GitHub Models is gone.** A GitHub-hosted runner calling `https://models.github.ai/catalog/models`
  and `/inference/chat/completions` with `permissions: models: read` gets `HTTP 200`, `content-type: text/plain`,
  body `OK` (4 bytes) in ~10 ms - no model output. `models.inference.ai.azure.com` no longer resolves.
  docs.github.com/en/github-models states: "As of July 30, 2026, GitHub Models has been fully retired."
  Search results that still describe it as a free CI LLM endpoint are stale. Consequence: multi-LLM
  validation cannot use a token-free hosted endpoint; it uses open-weight models run locally on the
  runner CPU (which also matches the project's "local LLM, CPU only" constraint), plus any
  OpenAI-compatible endpoint the user supplies a key for.
* The sandbox reaches only github.com / api.github.com / codeload.github.com / PyPI / npm. The Rust
  toolchain was obtained by a workflow that downloads the official tarballs on a runner, verifies
  SHA-256 against the official channel manifest and publishes them on a hidden git ref
  (`tools/dev/bootstrap-sandbox.sh`).
* **Open-weight models on a hosted runner (measured, run 43582226851, 2 cores, CPU only,
  Ollama 0.40.16, fixed 3.4K-token prompt).** The installer leaves `ollama.service`
  *disabled and inactive* while printing "The Ollama API is now available"; the server has
  to be started explicitly (`ollama serve`). The first probe (run 43581446661) measured
  nothing for this reason and still reported success; the probe now fails honestly.

  | model | size | prompt eval | generation | first request | repeat (cached prefix) |
  |---|---|---|---|---|---|
  | qwen2.5-coder:1.5b | 986 MB | 28.7 tok/s | 9.0 tok/s | 122 s | 1.6 s |
  | qwen2.5-coder:3b | 1.9 GB | 14.2 tok/s | 4.8 tok/s | 248 s | 2.6 s |
  | llama3.2:3b | 2.0 GB | 14.2 tok/s | 4.2 tok/s | 212 s | 3.3 s |
  | gemma3:4b | 3.4 GB | 13.2 tok/s | 4.4 tok/s | 310 s | 2.8 s |
  | qwen2.5-coder:7b | 4.7 GB | 6.4 tok/s | 2.2 tok/s | 608 s | 5.4 s |

  Consequences: prefix caching works, so a fixed language reference is paid for once per
  model; the reference must be short; 7B models are too slow for more than a few samples.
* **A keyless hosted endpoint exists** (`https://text.pollinations.ai/openai`, anonymous tier):
  it serves one model, GPT-OSS 20B (open weights, a reasoning model), and answered a
  3.4K-token request in 23 s. It is a third party with unknown limits and retention, is not
  reproducible, and is used only as an optional extra backend that receives the language
  reference and scene descriptions.

## F. Denoising (checked 2026-11-14)

Used by `crates/vl-render/src/denoise.rs`.

- H. Dammertz, D. Sewtz, J. Hanika, H. Lensch, "Edge-Avoiding A-Trous Wavelet Transform for
  fast Global Illumination Filtering", Proceedings of High Performance Graphics 2010
  (programme of HPG 2010, session "Ray Tracing I":
  https://www.highperformancegraphics.org/previous/www_2010/program.html ; bibliographic
  record: https://www.bibsonomy.org/bibtex/b88e0379e6de3fab91c0b81d697d01a4). Read only as
  a bibliographic record and through the SVGF paper's description; the paper itself was
  not fetched.
- C. Schied, A. Kaplanyan, C. Wyman, A. Patney, C. R. A. Chaitanya, J. Burgess, S. Liu,
  C. Dachsbacher, A. Lefohn, M. Salvi, "Spatiotemporal Variance-Guided Filtering: Real-Time
  Reconstruction for Path-Traced Global Illumination", HPG 2017
  (https://cg.ivd.kit.edu/publications/2017/svgf/svgf_preprint.pdf ,
  https://cwyman.org/papers/hpg17_svgf.pdf). Taken from the text of the preprint as
  returned by a search: the normal weight is max(0, n(p).n(q))^sigma_n; the luminance
  weight is exp(-|l(p) - l(q)| / (sigma_l * sqrt(g3x3(Var(l(p)))) + eps)); the 3x3 Gaussian
  prefilter of the variance "is only used to drive the luminance edge-stopping function,
  and it is not applied to the variance image propagated to the next iteration". The
  implementation follows these three statements. The values sigma_l = 4 and sigma_n = 64
  in the code were chosen here (the paper's own values were not in the text I saw).

