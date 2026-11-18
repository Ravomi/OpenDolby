"""Unit tests for OpenDolby DAX3 XML parsing, FIR synthesis, and PipeWire config generation."""

from pathlib import Path
import tempfile
import unittest
import numpy as np
from scipy.io import wavfile

from opendolby.cli import main as cli_main
from opendolby.dsp import synthesize_profile_irs
from opendolby.parser import parse_dax3_xml
from opendolby.pipewire import render_filter_chain_conf


XML_PATH = Path(__file__).resolve().parent.parent / "data" / "DEV_0294_SUBSYS_10431693_PCI_SUBSYS_16931043.xml"


class TestOpenDolby(unittest.TestCase):
    def test_parse_ux3404vc_xml(self) -> None:
        dev = parse_dax3_xml(XML_PATH)
        self.assertEqual(dev.sample_rate, 48000)
        self.assertEqual(dev.channels, 2)
        self.assertEqual(len(dev.band_freqs), 20)
        self.assertIn("music", dev.profiles)
        self.assertIn("movie", dev.profiles)
        self.assertIn("dynamic", dev.profiles)

        music = dev.profiles["music"]
        self.assertTrue(music.vlldp.audio_optimizer_enable)
        self.assertTrue(music.vlldp.speaker_peq_enable)
        self.assertGreater(len(music.vlldp.speaker_peq_filters), 0)
        self.assertEqual(len(music.vlldp.mb_compressor_bands), 4)

    def test_synthesize_irs_and_pipewire_conf(self) -> None:
        dev = parse_dax3_xml(XML_PATH)
        with tempfile.TemporaryDirectory() as tmp:
            irs_path = Path(tmp) / "opendolby-music.irs"
            summary = synthesize_profile_irs(dev, "music", irs_path, num_taps=2048)
            self.assertTrue(irs_path.exists())
            self.assertEqual(summary.sample_rate, 48000)

            rate, data = wavfile.read(irs_path)
            self.assertEqual(rate, 48000)
            self.assertEqual(data.shape, (2048, 2))
            self.assertTrue(np.isfinite(data).all())
            self.assertLessEqual(float(np.max(np.abs(data))), 1.0)

            conf = render_filter_chain_conf(
                "music",
                irs_path,
                sample_rate=48000,
                target_sink="alsa_output.test.Speaker__sink",
                use_limiter=True,
            )
            self.assertIn("libpipewire-module-filter-chain", conf)
            self.assertIn(str(irs_path.resolve()), conf)
            self.assertIn("limiter_stereo", conf)
            self.assertIn("alsa_output.test.Speaker__sink", conf)

    def test_cli_build_status_uninstall_and_invalid_profile(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            out_dir = Path(tmp) / "dist"
            self.assertEqual(cli_main(["inspect", "--xml", str(XML_PATH)]), 0)
            self.assertEqual(
                cli_main(["build", "music", "--xml", str(XML_PATH), "--out-dir", str(out_dir), "--target-sink", "test.sink"]),
                0,
            )
            self.assertTrue((out_dir / "opendolby-music.irs").exists())
            self.assertTrue((out_dir / "opendolby-music.conf").exists())
            self.assertEqual(cli_main(["status", "--conf-dir", str(out_dir)]), 0)
            self.assertEqual(cli_main(["uninstall", "--conf-dir", str(out_dir), "--no-restart"]), 0)
            self.assertFalse((out_dir / "opendolby-music.conf").exists())
            with self.assertRaises(SystemExit):
                cli_main(["build", "nonexistent_profile", "--xml", str(XML_PATH), "--out-dir", str(out_dir)])


if __name__ == "__main__":
    unittest.main()
