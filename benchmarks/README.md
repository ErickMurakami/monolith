# Benchmarks

Run from a native PowerShell terminal at the repository root. Close Monolith and gather the baseline. Launch the release binary, then measure with the launcher visible. For a resident measurement, start with `--resident`, confirm that the shortcut works, and hide the launcher with Escape.

```powershell
.\benchmarks\measure.ps1 -Scenario baseline -Output artifacts/baseline.json
.\benchmarks\measure.ps1 -Scenario launcher -Output artifacts/launcher.json
.\benchmarks\measure.ps1 -Scenario resident -Output artifacts/resident.json
```

Keep the foreground workload, power plan and background applications consistent across runs. Repeat each scenario and report the range. Record context beside the local outputs. Datasets are ignored by Git.

Available memory and commit describe the whole system. Per-process working set includes shared pages. Private working set counts private resident pages. Private bytes measures private committed virtual memory. The process CPU counter uses one logical CPU as 100%; divide by the logical processor count for its share of total capacity.

This script measures sampled memory, CPU and I/O. Startup, input latency, GPU, energy and feature-specific scenarios require separate workloads and instrumentation.
