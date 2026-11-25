# Benchmarks

Run from a native PowerShell terminal at the repository root. Close Monolith and gather the baseline. Launch the release binary, then measure with the launcher visible. For a resident measurement, start with `--resident`, confirm that the shortcut works, and hide the launcher with Escape. For a Dock measurement, launch with `--dock` and leave Spotlight hidden.

```powershell
.\benchmarks\measure.ps1 -Scenario baseline -Output artifacts/baseline.json
.\benchmarks\measure.ps1 -Scenario launcher -Output artifacts/launcher.json
.\benchmarks\measure.ps1 -Scenario resident -Output artifacts/resident.json
.\benchmarks\measure.ps1 -Scenario dock -Output artifacts/dock.json
```

Keep the foreground workload, power plan and background applications consistent across runs. Repeat each scenario and report the range. Record context beside the local outputs. Datasets are ignored by Git.

Available memory and commit describe the whole system. Per-process working set includes shared pages. Private working set counts private resident pages. Private bytes measures private committed virtual memory. The process CPU counter uses one logical CPU as 100%; divide by the logical processor count for its share of total capacity.

GPU values describe the busiest WDDM GPU engine per process or across the system. The system figure adds process utilisation for each hardware engine and takes the highest engine value. These figures help compare samples and are independent of CPU measurements. A null value means no useful GPU counter data was available. The methodology follows the busiest-engine interpretation used by [Windows Task Manager](https://devblogs.microsoft.com/directx/gpus-in-the-task-manager/).

This script measures sampled memory, CPU, GPU and I/O. Startup latency, input latency and energy require separate workloads and instrumentation.
