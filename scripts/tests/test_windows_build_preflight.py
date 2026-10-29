"""Known package scripts use the user-authorized process-only policy."""
import contextlib
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import Mock, patch
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import build_windows_current as windows


class WindowsCurrentDeliveryTests(unittest.TestCase):
    """Drive the native entry with a synthetic compiler, never real Windows input."""
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix='weavelo-windows-current-')
        self.addCleanup(temporary.cleanup)
        self.repo = Path(temporary.name).resolve()
        (self.repo/'scripts').mkdir()
        (self.repo/'VERSION').write_text('2.0.0-alpha.9')
        self.target = self.repo/'target'
        (self.target/'release').mkdir(parents=True)
        (self.target/'release/weavelo.exe').write_bytes(b'synthetic compiler output')
        self.ffmpeg = self.repo/'ffmpeg.exe'
        self.ffmpeg.write_bytes(b'synthetic media dependency')
        self.source = dict(commit='a'*40, git_tree='b'*40, state='clean',
                           patch_sha256=None, snapshot_sha256='c'*64, file_count=1)
        stack = contextlib.ExitStack()
        self.addCleanup(stack.close)
        stack.enter_context(patch.object(windows, '__file__', str(self.repo/'scripts/build_windows_current.py')))
        stack.enter_context(patch.object(sys, 'platform', 'win32'))
        stack.enter_context(patch.object(windows.gate, 'git', return_value=b'main'))
        stack.enter_context(patch.object(windows.gate, 'build_environment', return_value={'PATH':'fixture'}))
        stack.enter_context(patch.object(windows.gate, 'toolchain_digest', return_value='d'*64))
        stack.enter_context(patch.object(windows.gate, 'environment_digest', return_value='e'*64))
        stack.enter_context(patch.object(windows.gate, 'source_snapshot', return_value=self.source))
        self.product = stack.enter_context(patch.object(windows.gate, 'product_info',
            return_value={'desktop_identity':windows.desktop.identity()}))
        stack.enter_context(patch.object(windows.subprocess, 'check_output',
            return_value=json.dumps({'target_directory':str(self.target)}).encode()))
        stack.enter_context(patch.object(windows.subprocess, 'CREATE_NEW_PROCESS_GROUP', 512, create=True))
        self.compiler = stack.enter_context(patch.object(windows.subprocess, 'Popen',
            return_value=Mock(wait=Mock(return_value=0), poll=Mock(return_value=0))))
        self.preflight = stack.enter_context(patch.object(windows, 'packaging_preflight'))
        self.package = stack.enter_context(patch.object(windows.subprocess, 'run'))
        self.receipt = self.target/'package/windows/windows-current-build-receipt.json'

    def run_entry(self, *options):
        args = ['build_windows_current.py','--expected-commit','a'*40,
                '--ffmpeg',str(self.ffmpeg),*options]
        output = io.StringIO()
        with patch.object(sys, 'argv', args), contextlib.redirect_stdout(output):
            windows.main()
        return json.loads(output.getvalue())

    def test_independent_compiler_output_routes_only_to_desktop_package(self):
        result = self.run_entry()
        command = self.package.call_args.args[0]
        self.assertEqual(command[command.index('-ExecutionPolicy')+1], 'RemoteSigned')
        self.assertIn('-Desktop', command)
        self.assertIn('-SkipBuild', command)
        self.assertEqual(command[command.index('-BuildReceipt')+1], str(self.receipt))
        self.assertEqual(result['package'], str(self.target/'package/windows/desktop/Weavelo-Windows-x64.zip'))
        self.assertEqual(json.loads(self.receipt.read_text())['build_identity']['source']['commit'], 'a'*40)
        self.assertFalse((self.target/'package/windows/Weavelo-Windows-x64.zip').exists())

    def test_retained_or_unknown_namespace_cannot_mint_current_receipt(self):
        for namespace in (None, {'application_id':'fun.hackerlife.weavelo'}):
            with self.subTest(namespace=namespace):
                self.product.return_value = {'desktop_identity':namespace}
                with self.assertRaisesRegex(windows.gate.IdentityRejected, 'independent Desktop namespace'):
                    self.run_entry()
                self.package.assert_not_called()
                self.assertFalse(self.receipt.exists())
                self.assertFalse((self.target/'.weavelo-windows-current.lock').exists())

    def test_build_only_keeps_native_receipt_without_invoking_package_policy(self):
        result = self.run_entry('--build-only')
        self.assertEqual(result['stage'], 'build-only')
        self.assertFalse(result['package_generated'])
        self.assertTrue(self.receipt.is_file())
        self.preflight.assert_not_called()
        self.package.assert_not_called()
        self.assertEqual(self.compiler.call_count, 1)


class WindowsBuildEnvironmentTests(unittest.TestCase):
    def test_developer_prompt_survives_filter_without_private_or_policy_values(self):
        prompt = {
            'PATH': r'C:\VS\bin',
            'INCLUDE': r'C:\VS\include',
            'LIB': r'C:\VS\lib',
            'VISUALSTUDIOVERSION': '17.0',
            'VSINSTALLDIR': r'C:\VS',
            'VCTOOLSINSTALLDIR': r'C:\VS\VC\Tools\MSVC\14.44',
            'WINDOWSSDKDIR': r'C:\SDK',
            'WINDOWSSDKVERSION': '10.0.26100.0',
            'PROGRAMFILES(X86)': r'C:\Program Files (x86)',
            'PROGRAMDATA': r'C:\ProgramData',
            'SYSTEMDRIVE': 'C:',
            'VSCMD_ARG_TGT_ARCH': 'x64',
        }
        inherited = dict(prompt, PRIVATE_TOKEN='synthetic-secret',
                         PSEXECUTIONPOLICYPREFERENCE='Restricted')
        with patch.object(sys, 'platform', 'win32'), \
                patch.dict(os.environ, inherited, clear=True):
            self.assertEqual(windows.gate.build_environment(), prompt)
            self.assertEqual(windows.gate.compiler_environment_record(), prompt)
            self.assertEqual(dict(os.environ), inherited)

    def test_selected_visual_studio_and_sdk_are_part_of_build_identity(self):
        prompt = {'PATH': 'compiler', 'VISUALSTUDIOVERSION': '17.0',
                  'VSINSTALLDIR': 'vs-current', 'WINDOWSSDKVERSION': 'sdk-current',
                  'PROGRAMDATA': 'vs-instance-data', 'SYSTEMDRIVE': 'C:'}
        with patch.object(sys, 'platform', 'win32'), \
                patch.dict(os.environ, prompt, clear=True):
            before = windows.gate.environment_digest()
            for key in ('VISUALSTUDIOVERSION', 'VSINSTALLDIR', 'WINDOWSSDKVERSION',
                        'PROGRAMDATA', 'SYSTEMDRIVE'):
                with self.subTest(key=key), patch.dict(os.environ, {key: 'changed'}):
                    self.assertNotEqual(windows.gate.environment_digest(), before)

    def test_windows_prompt_fields_do_not_change_other_platform_identities(self):
        with patch.object(sys, 'platform', 'linux'), \
                patch.dict(os.environ, {'PATH': 'compiler'}, clear=True):
            before = windows.gate.environment_digest()
            with patch.dict(os.environ, {'VISUALSTUDIOVERSION': '17.0'}):
                self.assertEqual(windows.gate.build_environment(), {'PATH': 'compiler'})
                self.assertEqual(windows.gate.environment_digest(), before)

    @unittest.skipUnless(sys.platform == 'win32', 'Native Windows known-folder query')
    def test_native_msbuild_common_data_directory_survives_filter(self):
        admitted = {key: os.environ[key]
                    for key in windows.gate.BUILD_ENV + windows.gate.WINDOWS_BUILD_ENV
                    if key in os.environ}
        with patch.dict(os.environ, admitted, clear=True):
            filtered = windows.gate.build_environment()
        # MSBuild FileTracker initializes this through the same .NET API.
        result = subprocess.run(
            ['powershell.exe', '-NoProfile', '-Command',
             '[Console]::Write([Environment]::GetFolderPath('
             '[Environment+SpecialFolder]::CommonApplicationData))'],
            env=filtered, capture_output=True, text=True, timeout=30)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(result.stdout.strip(), 'MSBuild needs a nonempty common data path')
        self.assertTrue(Path(result.stdout.strip()).is_dir())


class WindowsPackagingPreflightTests(unittest.TestCase):
    def report(self, policy='RemoteSigned', zone=0, signature='NotSigned'):
        return dict(effective=policy, scripts=[dict(path=name, signature=signature, zone=zone)
                    for name in ('package_windows_unified.ps1','verify_desktop_gui.ps1')])

    def test_no_inherited_policy_is_not_a_policy_grant(self):
        original={'PATH':'existing'}
        self.assertEqual(windows.packaging_environment(original,{}),original)
        self.assertNotIn('PSExecutionPolicyPreference',original)

    def test_preserves_only_previously_chosen_process_policy(self):
        original={'PATH':'existing'}; inherited={'PSExecutionPolicyPreference':'RemoteSigned'}
        result=windows.packaging_environment(original,inherited)
        self.assertEqual(result,dict(original,PSExecutionPolicyPreference='RemoteSigned'))
        self.assertEqual(original,{'PATH':'existing'})
        self.assertEqual(inherited,{'PSExecutionPolicyPreference':'RemoteSigned'})

    def test_packaging_preserves_native_command_extensions_without_changing_compiler_paths(self):
        original={'PATH':'compiler-path','LIB':'compiler-libraries'}
        inherited={'PATHEXT':'.COM;.EXE;.BAT;.CMD','PATH':'another-path','PRIVATE_TOKEN':'synthetic-secret'}
        result=windows.packaging_environment(original,inherited)
        self.assertEqual(result,dict(original,PATHEXT=inherited['PATHEXT']))
        self.assertEqual(original,{'PATH':'compiler-path','LIB':'compiler-libraries'})

    def test_restricted_is_rejected_without_a_fallback(self):
        with self.assertRaisesRegex(windows.gate.IdentityRejected,'Restricted'):
            windows.validate_packaging_policy(self.report('Restricted'))

    def test_signed_policy_and_internet_zone_remain_enforced(self):
        for report in (self.report('AllSigned'),self.report(zone=3)):
            with self.assertRaises(windows.gate.IdentityRejected):windows.validate_packaging_policy(report)
        windows.validate_packaging_policy(self.report(zone=3,signature='Valid'))
        windows.validate_packaging_policy(self.report())

    def test_incomplete_observation_is_not_success(self):
        for report in ({'effective':'RemoteSigned','scripts':[]},self.report(zone='0')):
            with self.assertRaises(windows.gate.IdentityRejected):windows.validate_packaging_policy(report)

    def test_policy_refusal_occurs_before_source_scan_or_compiler(self):
        with patch.object(sys,'argv',['build_windows_current.py','--expected-commit','a'*40,'--ffmpeg','C:/private/ffmpeg.exe']), \
             patch.object(sys,'platform','win32'), \
             patch.object(windows.gate,'git',return_value=b'main'), \
             patch.object(windows.gate,'build_environment',return_value={}), \
             patch.object(windows,'packaging_preflight',side_effect=windows.gate.IdentityRejected('Restricted')), \
             patch.object(windows.gate,'source_snapshot') as source, \
             patch.object(windows.subprocess,'Popen') as compiler:
            with self.assertRaisesRegex(windows.gate.IdentityRejected,'Restricted'):windows.main()
            source.assert_not_called();compiler.assert_not_called()

    def test_probe_reads_policy_and_metadata_without_running_script(self):
        with tempfile.TemporaryDirectory() as folder:
            root=Path(folder).resolve();(root/'scripts').mkdir()
            for name in ('package_windows_unified.ps1','verify_desktop_gui.ps1'):(root/'scripts'/name).write_text('param()')
            response=subprocess.CompletedProcess([],0,json.dumps(self.report()),'')
            with patch.object(windows.subprocess,'run',return_value=response) as run:
                windows.packaging_preflight(root,{'PATH':'existing'})
            argv=run.call_args.args[0]
            self.assertEqual(argv[:5],['powershell.exe','-NoProfile','-ExecutionPolicy','RemoteSigned','-Command'])
            for forbidden in ('Set-ExecutionPolicy','Unblock-File'):
                self.assertNotIn(forbidden,argv[5])
            self.assertNotIn('-File',argv)
