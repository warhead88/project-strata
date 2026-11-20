"""Measure the complete Strata process on macOS, by camera leg."""
import ctypes as c
import subprocess
import sys
import time


class Usage(c.Structure):
    _fields_ = [
        ("uuid", c.c_uint8 * 16),
        *[(field, c.c_uint64) for field in (
            "user_time", "system_time", "pkg_idle_wakeups", "interrupt_wakeups",
            "page_ins", "wired_bytes", "rss_bytes", "footprint_bytes",
            "start", "exit",
        )],
    ]


class Timebase(c.Structure):
    _fields_ = [("numer", c.c_uint32), ("denom", c.c_uint32)]


def sample(pid, proc_pid_rusage):
    usage = Usage()
    if proc_pid_rusage(pid, 0, c.byref(usage)) != 0:
        raise OSError(c.get_errno(), f"proc_pid_rusage({pid})")
    return time.monotonic_ns(), usage


def main():
    if sys.platform != "darwin":
        sys.exit("profile.py uses macOS process accounting")
    cpus = int(subprocess.check_output(["sysctl", "-n", "hw.logicalcpu"], text=True))
    proc_pid_rusage = c.CDLL("/usr/lib/libproc.dylib", use_errno=True).proc_pid_rusage
    proc_pid_rusage.argtypes = (c.c_int, c.c_int, c.POINTER(Usage))
    proc_pid_rusage.restype = c.c_int
    info = Timebase()
    if c.CDLL(None).mach_timebase_info(c.byref(info)) != 0 or info.denom == 0:
        raise RuntimeError("could not read Mach timebase")
    print(f"terrain_profile logical_cpus={cpus} mach_timebase={info.numer}/{info.denom}", flush=True)
    with subprocess.Popen(
        ["/usr/bin/time", "-l", *sys.argv[1:], "--terrain-flight"],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    ) as parent:
        active = {}
        for line in parent.stdout:
            print(line, end="", flush=True)
            fields = line.split()
            if fields[:2] == ["terrain_flight", "pid"]:
                pid = int(fields[2])
            if fields[:2] == ["terrain_flight", "begin"]:
                active[fields[2]] = sample(pid, proc_pid_rusage)
            if fields[:2] == ["terrain_flight", "end"]:
                name = fields[2]
                start_ns, start = active.pop(name)
                end_ns, end = sample(pid, proc_pid_rusage)
                cpu_ticks = (end.user_time + end.system_time) - (start.user_time + start.system_time)
                cpu = cpu_ticks * info.numer / info.denom
                wall = end_ns - start_ns
                print(f"terrain_profile {name} cpu_seconds={cpu / 1e9:.4f} "
                      f"wall_seconds={wall / 1e9:.4f} machine_cpu_pct={100 * cpu / wall / cpus:.3f} "
                      f"rss_mib={end.rss_bytes / 2**20:.1f} footprint_mib={end.footprint_bytes / 2**20:.1f}",
                      flush=True)
    return parent.returncode


if __name__ == "__main__":
    sys.exit(main())
