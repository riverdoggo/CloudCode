from __future__ import annotations

import subprocess
import sys
import unittest


class ApiContractTests(unittest.TestCase):
    def test_remote_mode_clis_run(self) -> None:
        remote_result = subprocess.run([sys.executable, '-m', 'src.main', 'remote-mode', 'workspace'], check=True, capture_output=True, text=True)
        ssh_result = subprocess.run([sys.executable, '-m', 'src.main', 'ssh-mode', 'workspace'], check=True, capture_output=True, text=True)
        teleport_result = subprocess.run([sys.executable, '-m', 'src.main', 'teleport-mode', 'workspace'], check=True, capture_output=True, text=True)
        self.assertIn('mode=remote', remote_result.stdout)
        self.assertIn('mode=ssh', ssh_result.stdout)
        self.assertIn('mode=teleport', teleport_result.stdout)

    def test_setup_report_and_registry_filters_run(self) -> None:
        setup_result = subprocess.run(
            [sys.executable, '-m', 'src.main', 'setup-report'],
            check=True,
            capture_output=True,
            text=True,
        )
        command_result = subprocess.run(
            [sys.executable, '-m', 'src.main', 'commands', '--limit', '5', '--no-plugin-commands'],
            check=True,
            capture_output=True,
            text=True,
        )
        tool_result = subprocess.run(
            [sys.executable, '-m', 'src.main', 'tools', '--limit', '5', '--simple-mode', '--no-mcp'],
            check=True,
            capture_output=True,
            text=True,
        )
        self.assertIn('Setup Report', setup_result.stdout)
        self.assertIn('Command entries:', command_result.stdout)
        self.assertIn('Tool entries:', tool_result.stdout)

    def test_setup_report_mentions_deferred_init(self) -> None:
        result = subprocess.run(
            [sys.executable, '-m', 'src.main', 'setup-report'],
            check=True,
            capture_output=True,
            text=True,
        )
        self.assertIn('Deferred init:', result.stdout)
        self.assertIn('plugin_init=True', result.stdout)


if __name__ == '__main__':
    unittest.main()
