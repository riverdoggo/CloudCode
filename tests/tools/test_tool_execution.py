from __future__ import annotations

import subprocess
import sys
import unittest

from src.commands import PORTED_COMMANDS
from src.tools import PORTED_TOOLS


class ToolExecutionTests(unittest.TestCase):
    def test_command_and_tool_snapshots_are_nontrivial(self) -> None:
        self.assertGreaterEqual(len(PORTED_COMMANDS), 150)
        self.assertGreaterEqual(len(PORTED_TOOLS), 100)

    def test_commands_and_tools_cli_run(self) -> None:
        commands_result = subprocess.run(
            [sys.executable, '-m', 'src.main', 'commands', '--limit', '5', '--query', 'review'],
            check=True,
            capture_output=True,
            text=True,
        )
        tools_result = subprocess.run(
            [sys.executable, '-m', 'src.main', 'tools', '--limit', '5', '--query', 'MCP'],
            check=True,
            capture_output=True,
            text=True,
        )
        self.assertIn('Command entries:', commands_result.stdout)
        self.assertIn('Tool entries:', tools_result.stdout)

    def test_exec_command_and_tool_cli_run(self) -> None:
        command_result = subprocess.run(
            [sys.executable, '-m', 'src.main', 'exec-command', 'review', 'inspect security review'],
            check=True,
            capture_output=True,
            text=True,
        )
        tool_result = subprocess.run(
            [sys.executable, '-m', 'src.main', 'exec-tool', 'MCPTool', 'fetch resource list'],
            check=True,
            capture_output=True,
            text=True,
        )
        self.assertIn("Mirrored command 'review'", command_result.stdout)
        self.assertIn("Mirrored tool 'MCPTool'", tool_result.stdout)

    def test_tool_permission_filtering_cli_runs(self) -> None:
        result = subprocess.run(
            [sys.executable, '-m', 'src.main', 'tools', '--limit', '10', '--deny-prefix', 'mcp'],
            check=True,
            capture_output=True,
            text=True,
        )
        self.assertIn('Tool entries:', result.stdout)
        self.assertNotIn('MCPTool', result.stdout)

    def test_command_graph_and_tool_pool_cli_run(self) -> None:
        command_graph = subprocess.run([sys.executable, '-m', 'src.main', 'command-graph'], check=True, capture_output=True, text=True)
        tool_pool = subprocess.run([sys.executable, '-m', 'src.main', 'tool-pool'], check=True, capture_output=True, text=True)
        self.assertIn('Command Graph', command_graph.stdout)
        self.assertIn('Tool Pool', tool_pool.stdout)


if __name__ == '__main__':
    unittest.main()
