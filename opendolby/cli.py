"""Command-line interface for OpenDolby."""

from __future__ import annotations

import argparse
from pathlib import Path

from opendolby.dsp import synthesize_profile_irs
from opendolby.parser import parse_dax3_xml
from opendolby.pipewire import (
    CONF_PREFIX,
    DEFAULT_CONF_DIR,
    detect_speaker_sink,
    render_filter_chain_conf,
    restart_pipewire,
    uninstall_profiles,
)


DEFAULT_XML = Path(__file__).resolve().parent.parent / "data" / "DEV_0294_SUBSYS_10431693_PCI_SUBSYS_16931043.xml"


def _cmd_inspect(args: argparse.Namespace) -> None:
    dev = parse_dax3_xml(args.xml)
    print(f"XML: {dev.xml_path}")
    print(f"Version: {dev.xml_version}  Date: {dev.tuning_date}")
    print(f"Endpoint: {dev.endpoint_type}  Rate: {dev.sample_rate} Hz  Channels: {dev.channels}")
    print(f"Profiles ({len(dev.profiles)}):")
    for name, p in dev.profiles.items():
        peq_count = sum(1 for f in p.vlldp.speaker_peq_filters if f.enabled)
        mb_count = len(p.vlldp.mb_compressor_bands) if p.vlldp.mb_compressor_enable else 0
        print(
            f"  - {name:20s} vlldp_opt={p.vlldp.audio_optimizer_enable} "
            f"peq={peq_count} mb={mb_count} ieq={p.cp.ieq_enable}"
        )


def _cmd_build(args: argparse.Namespace) -> None:
    dev = parse_dax3_xml(args.xml)
    out_dir = Path(args.out_dir)
    out_dir.mkdir(parents=True, exist_ok=True)
    target_sink = args.target_sink or detect_speaker_sink()
    profiles = list(dev.profiles) if args.profile == "all" else [args.profile]

    for prof in profiles:
        irs_path = out_dir / f"{CONF_PREFIX}{prof}.irs"
        conf_path = out_dir / f"{CONF_PREFIX}{prof}.conf"
        summary = synthesize_profile_irs(dev, prof, irs_path, num_taps=args.taps)
        conf_text = render_filter_chain_conf(
            profile_name=prof,
            irs_path=irs_path,
            sample_rate=dev.sample_rate,
            target_sink=target_sink,
            use_limiter=not args.no_limiter,
        )
        conf_path.write_text(conf_text, encoding="utf-8")
        print(
            f"Built {prof}: {conf_path} + {irs_path} "
            f"(peak={summary.peak_gain_db:+.2f} dB, trim={summary.headroom_trim_db:+.2f} dB)"
        )


def _cmd_install(args: argparse.Namespace) -> None:
    dev = parse_dax3_xml(args.xml)
    conf_dir = Path(args.conf_dir)
    conf_dir.mkdir(parents=True, exist_ok=True)
    uninstall_profiles(conf_dir)

    irs_path = conf_dir / f"{CONF_PREFIX}{args.profile}.irs"
    conf_path = conf_dir / f"{CONF_PREFIX}{args.profile}.conf"
    target_sink = args.target_sink or detect_speaker_sink()

    summary = synthesize_profile_irs(dev, args.profile, irs_path, num_taps=args.taps)
    conf_text = render_filter_chain_conf(
        profile_name=args.profile,
        irs_path=irs_path,
        sample_rate=dev.sample_rate,
        target_sink=target_sink,
        use_limiter=not args.no_limiter,
    )
    conf_path.write_text(conf_text, encoding="utf-8")
    print(f"Installed {conf_path} (trim={summary.headroom_trim_db:+.2f} dB)")
    if not args.no_restart:
        restart_pipewire()
        print("Restarted PipeWire and WirePlumber.")


def _cmd_uninstall(args: argparse.Namespace) -> None:
    removed = uninstall_profiles(Path(args.conf_dir))
    for p in removed:
        print(f"Removed {p}")
    if removed and not args.no_restart:
        restart_pipewire()
        print("Restarted PipeWire and WirePlumber.")
    elif not removed:
        print("No OpenDolby PipeWire configs found.")


def build_parser() -> argparse.ArgumentParser:
    """Create the CLI argument parser."""
    parser = argparse.ArgumentParser(prog="opendolby", description="OpenDolby DAX3 to PipeWire tool")
    sub = parser.add_subparsers(dest="command", required=True)

    p_inspect = sub.add_parser("inspect", help="Inspect a DAX3 XML file and list profiles")
    p_inspect.add_argument("--xml", default=str(DEFAULT_XML), help="Path to DAX3 XML")
    p_inspect.set_defaults(func=_cmd_inspect)

    p_build = sub.add_parser("build", help="Generate .irs and PipeWire .conf into an output directory")
    p_build.add_argument("profile", help="Profile name (e.g. music, movie, dynamic) or 'all'")
    p_build.add_argument("--xml", default=str(DEFAULT_XML), help="Path to DAX3 XML")
    p_build.add_argument("--out-dir", default="dist", help="Output directory")
    p_build.add_argument("--target-sink", default=None, help="Target ALSA sink node.name")
    p_build.add_argument("--taps", type=int, default=4096, help="FIR tap count")
    p_build.add_argument("--no-limiter", action="store_true", help="Disable LSP stereo limiter node")
    p_build.set_defaults(func=_cmd_build)

    p_install = sub.add_parser("install", help="Install a profile into PipeWire conf.d")
    p_install.add_argument("profile", help="Profile name (e.g. music, movie, dynamic)")
    p_install.add_argument("--xml", default=str(DEFAULT_XML), help="Path to DAX3 XML")
    p_install.add_argument("--conf-dir", default=str(DEFAULT_CONF_DIR), help="PipeWire conf.d path")
    p_install.add_argument("--target-sink", default=None, help="Target ALSA sink node.name")
    p_install.add_argument("--taps", type=int, default=4096, help="FIR tap count")
    p_install.add_argument("--no-limiter", action="store_true", help="Disable LSP stereo limiter node")
    p_install.add_argument("--no-restart", action="store_true", help="Skip restarting PipeWire")
    p_install.set_defaults(func=_cmd_install)

    p_uninst = sub.add_parser("uninstall", help="Remove installed OpenDolby PipeWire configs")
    p_uninst.add_argument("--conf-dir", default=str(DEFAULT_CONF_DIR), help="PipeWire conf.d path")
    p_uninst.add_argument("--no-restart", action="store_true", help="Skip restarting PipeWire")
    p_uninst.set_defaults(func=_cmd_uninstall)

    return parser


def main(argv: list[str] | None = None) -> int:
    """CLI entry point."""
    parser = build_parser()
    args = parser.parse_args(argv)
    args.func(args)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
