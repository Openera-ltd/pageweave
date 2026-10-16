# Hardware Support Policy

Do not publish minimum/recommended hardware claims until tested. Build a matrix of actual Windows versions, CPU instruction sets, RAM, GPU vendor/driver/VRAM, storage and display DPI. At minimum, test CPU-only mode, integrated GPU, the minimum claimed RAM, recommended hardware, high DPI and long-session behavior. Include low-disk, OOM, GPU-loss and thermal-throttling scenarios. Record app startup, view rendering, peak RAM/VRAM, conversion/OCR/TTS throughput, frame-time and recovery. Each claimed supported configuration must pass core open/save/view and declared feature tests.

Feature support may vary by installed resource, OS and hardware. Capability availability is declared through the resource/runtime registry; never infer it from GPU presence alone. Any CPU fallback must be explicitly tested. Keep raw benchmark artifacts and build/driver details.
