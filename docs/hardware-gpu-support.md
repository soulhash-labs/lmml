# LMML AI GPU Support Catalog

LMML keeps a built-in AI GPU catalog in `lmml-detect` so the Detect tab can
turn raw device names into practical local-AI guidance.

The catalog is advisory. It does not download models, install drivers, or claim
that every listed accelerator is fully validated on this machine. It helps users
understand likely backend choice, VRAM class, and local model scale.

## Backend Priority

For local llama.cpp inference, the practical priority is:

1. NVIDIA CUDA
2. AMD ROCm/HIP, when the exact card and distro are supported
3. AMD Vulkan/RADV for unsupported or appliance-style AMD boards such as BC-250
4. Intel oneAPI/OpenVINO/XPU, or Intel Gaudi for datacenter accelerators
5. Vulkan or CPU fallback for unsupported GPU stacks

VRAM remains the first sizing constraint for local AI. Ecosystem support is the
second constraint.

Current lmml ROCm/HIP support is conservative: `lmml-detect` probes
`hipconfig` and `rocminfo`, auto-selects HIP only when at least one `gfx*` target
is visible, and `lmml-build` emits upstream llama.cpp flags
`-DGGML_HIP=ON -DGPU_TARGETS=...`. Operators can still force Vulkan for AMD
cards that have Mesa/RADV support but do not have a clean ROCm stack.

## NVIDIA CUDA

| Class | GPU | Nominal Memory | LMML Guidance |
| --- | --- | ---: | --- |
| Datacenter | Blackwell B200 Tensor Core GPU | 180GB HBM3e | frontier training / massive inference |
| Datacenter | H200 Tensor Core GPU | 141GB HBM3e | enterprise LLM workhorse |
| Workstation | RTX PRO 6000 Blackwell | 96GB GDDR7 ECC | best single-card workstation tier |
| Prosumer | GeForce RTX 5090 | 32GB GDDR7 | 30B-70B quantized local AI |
| Prosumer | GeForce RTX 4090 | 24GB GDDR6X | excellent used-market 24GB CUDA card |
| Prosumer | GeForce RTX 3090 | 24GB GDDR6X | great-value 24GB prosumer card |
| Mid-range | GeForce RTX 5080 | 16GB GDDR7 | 8B-14B high-throughput local AI |
| Mid-range | GeForce RTX 5060 Ti 16GB | 16GB GDDR7 | budget 16GB CUDA sweet spot |
| Mid-range | GeForce RTX 4060 Ti 16GB | 16GB GDDR6 | efficient 8B-14B models |
| Entry | GeForce RTX 5070 | 12GB GDDR7 | 7B-12B quantized local AI |
| Entry | GeForce RTX 3060 12GB | 12GB GDDR6 | cheapest native CUDA entry tier |
| Entry | GeForce RTX 5060 | 8GB GDDR7 | small 7B quantized models |

## AMD ROCm/HIP

| Class | GPU | Nominal Memory | LMML Guidance |
| --- | --- | ---: | --- |
| Datacenter | Instinct MI355X | 288GB HBM3E | massive ROCm memory capacity |
| Datacenter | Instinct MI300X | 192GB HBM3 | large-memory ROCm datacenter tier |
| Workstation | Radeon AI PRO R9700 | 32GB GDDR6 | 32GB workstation ROCm card |
| Prosumer | Radeon RX 7900 XTX | 24GB GDDR6 | 24GB VRAM-per-dollar ROCm card |
| Mid-range | Radeon RX 9070 XT | 16GB GDDR6 | current-gen 16GB ROCm option |
| Mid-range | Radeon RX 9060 XT 16GB | 16GB GDDR6 | affordable 16GB RDNA 4 option |
| Headless appliance | BC-250 / Cyan Skillfish | 16GB unified GDDR6 | Qwen 9B Q4 Vulkan LAN node |

Note: the provided `RX 960 XT` name is treated as an alias for AMD's official
`RX 9060 XT` product name.

BC-250 is treated as a Vulkan/RADV target rather than a ROCm-first target. It is
a PS5-derived RDNA 2 board with unified CPU/GPU GDDR6 memory, so practical LMML
support is headless serving with conservative context first: Qwen3.5 9B Q4_K_M,
`-ngl 99`, `-fa`, single slot, and 4096 context until the exact BIOS memory
split, Mesa version, cooling, and power limits are validated.

## Intel

| Class | Accelerator | Nominal Memory | LMML Guidance |
| --- | --- | ---: | --- |
| Datacenter | Gaudi 3 AI Accelerator | 128GB HBM2e | Ethernet-scale AI accelerator |
| Workstation | Arc Pro B70 | 32GB GDDR6 ECC | affordable 32GB workstation tier |
| Entry | Arc A770 | 16GB GDDR6 | discount 16GB entry tier |
| Entry | Arc B580 | 12GB GDDR6 | budget 12GB entry tier |
| Integrated | Core Ultra NPU | shared memory | very light local AI only |

## What LMML Does With This Catalog

- Displays matched accelerator guidance in the Detect tab.
- Matches raw CUDA names from `nvidia-smi`.
- Matches Vulkan summary device names for non-NVIDIA GPUs when available.
- Matches Intel Core Ultra CPU model strings for integrated NPU guidance.
- Keeps backend support advisory: ROCm, oneAPI/OpenVINO, Gaudi, and Vulkan still
  need host drivers and runtime validation outside this static catalog.

## Source Anchors

- NVIDIA GeForce RTX 50 Series:
  <https://www.nvidia.com/en-us/geforce/graphics-cards/50-series/>
- NVIDIA H200:
  <https://www.nvidia.com/en-us/data-center/h200/>
- NVIDIA RTX PRO 6000 Blackwell:
  <https://www.nvidia.com/en-us/data-center/rtx-pro-6000-blackwell-server-edition/>
- NVIDIA DGX B200:
  <https://docs.nvidia.com/dgx/dgxb200-user-guide/introduction-to-dgxb200.html>
- AMD Instinct MI355X:
  <https://www.amd.com/en/products/accelerators/instinct/mi350/mi355x.html>
- AMD Radeon AI PRO:
  <https://www.amd.com/en/products/graphics/workstations/radeon-ai-pro.html>
- AMD Radeon RX 9070 XT:
  <https://www.amd.com/en/products/graphics/desktops/radeon/9000-series/amd-radeon-rx-9070xt.html>
- AMD Radeon RX 9060 XT:
  <https://www.amd.com/en/products/graphics/desktops/radeon/9000-series/amd-radeon-rx-9060xt.html>
- AMD BC250 hardware specifications:
  <https://elektricm.github.io/amd-bc250-docs/hardware/specifications/>
- AMD BC250 Debian/PikaOS setup:
  <https://elektricm.github.io/amd-bc250-docs/linux/debian/>
- Intel Arc Pro B-Series:
  <https://www.intel.com/content/www/us/en/products/docs/discrete-gpus/arc/workstations/b-series/overview.html>
- Intel Arc Pro B70:
  <https://www.intel.com/content/www/us/en/products/sku/245797/intel-arc-pro-b70-graphics/specifications.html>
- Intel Gaudi 3:
  <https://cdrdv2-public.intel.com/845118/gaudi-3-ai-accelerator-30-3-30.pdf>
