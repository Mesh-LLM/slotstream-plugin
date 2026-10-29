# Third-party notices

`Sources/Slotstream/VQKernelSources.swift` contains Metal kernel strings from
[VQLab](https://github.com/noahzelezny/VQLab), by Noah Zelezny and contributors,
under the [Apache License 2.0](Licenses/VQLab-Apache-2.0.txt).

The strings were extracted from the pinned Flash Next VQ runtime whose digest
is recorded in `Tools/vq_kernel_sources.py`. Their bytes also match
`src/vqlab/runtime/vq_switch.py` at VQLab commit
`97b4380c60f5faf55bdc777102d042468c4644ca`. The modification is packaging those
constants in Swift; runtime wrappers and bounded diagnostics are Slotstream
code. No upstream Python is executed by the native runtime.

This code license does not license the separately distributed model weights.
Each model artifact retains its own license and provenance requirements.
