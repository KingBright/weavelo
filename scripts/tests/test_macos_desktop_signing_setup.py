"""Disposable setup fixtures; no real certificate, keychain or trust mutation."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import setup_macos_desktop_signing as setup


class DesktopSigningSetupTests(unittest.TestCase):
    def setUp(self):
        temporary=tempfile.TemporaryDirectory();self.addCleanup(temporary.cleanup)
        self.root=Path(temporary.name).resolve()/'signing'
        self.addCleanup(patch.stopall)
        patch.object(setup,'ROOT',self.root).start()
        patch.object(setup.sys,'platform','darwin').start()
        patch.object(setup.socket,'gethostname',return_value='MacStudio.local').start()

    def test_existing_partial_state_is_preserved_before_any_native_action(self):
        self.root.mkdir(mode=0o700);old=self.root/'creation-intent.json';old.write_text('retain')
        with patch.object(setup.subprocess,'run') as native:
            with self.assertRaisesRegex(setup.SetupRejected,'never regenerate'):setup.create(self.root)
        native.assert_not_called();self.assertEqual(old.read_text(),'retain')

    def test_same_project_certificate_without_receipt_cannot_be_duplicated(self):
        observation=subprocess.CompletedProcess([],0,'SHA-1 hash: '+'A'*40,'')
        with patch.object(setup.subprocess,'run',return_value=observation) as native:
            with self.assertRaisesRegex(setup.SetupRejected,'Existing Desktop certificate'):setup.create(self.root)
        self.assertEqual(native.call_count,1);self.assertFalse(self.root.exists())

    def test_creation_password_stays_in_pipe_and_only_public_record_survives(self):
        der=b'synthetic public certificate DER';fingerprint=hashlib.sha1(der).hexdigest();calls=[]
        def run(args,**kwargs):
            calls.append((args,kwargs))
            if 'req' in args:
                Path(args[args.index('-keyout')+1]).write_bytes(b'synthetic private fixture')
                Path(args[args.index('-out')+1]).write_bytes(b'synthetic public certificate PEM')
            elif 'pkcs12' in args:Path(args[args.index('-out')+1]).write_bytes(b'synthetic encrypted fixture')
            return subprocess.CompletedProcess(args,0,'','')
        with patch.object(setup.subprocess,'run',side_effect=run), \
             patch.object(setup.subprocess,'check_output',return_value=der), \
             patch.object(setup,'identities',return_value={fingerprint.upper()}), \
             patch.object(setup,'import_identity') as native_import:
            result=setup.create(self.root)
        secret=native_import.call_args.args[1]
        export=next((args,options) for args,options in calls if 'pkcs12' in args)
        self.assertEqual(export[1]['input'],(secret+'\n').encode())
        self.assertNotIn(secret,' '.join(str(args) for args,_ in calls))
        self.assertNotIn(secret,json.dumps(result))
        self.assertFalse((self.root/'.creation').exists())
        self.assertFalse(result['private_key_export_retained']);self.assertFalse(result['trust_settings_changed'])
        self.assertEqual(result['certificate_sha1'],fingerprint)
        self.assertEqual(result['scope'],'fun.hackerlife.weavelo.desktop')

    def test_import_failure_preserves_one_stage_and_cannot_generate_another(self):
        def run(args,**kwargs):
            if 'req' in args:
                Path(args[args.index('-keyout')+1]).write_bytes(b'synthetic private fixture')
                Path(args[args.index('-out')+1]).write_bytes(b'synthetic public cert')
            elif 'pkcs12' in args:Path(args[args.index('-out')+1]).write_bytes(b'synthetic envelope')
            return subprocess.CompletedProcess(args,0,'','')
        with patch.object(setup.subprocess,'run',side_effect=run), \
             patch.object(setup.subprocess,'check_output',return_value=b'public DER'), \
             patch.object(setup,'import_identity',side_effect=setup.SetupRejected('native denial')) as native:
            with self.assertRaisesRegex(setup.SetupRejected,'native denial'):setup.create(self.root)
            with self.assertRaisesRegex(setup.SetupRejected,'never regenerate'):setup.create(self.root)
        self.assertEqual(native.call_count,1);self.assertTrue((self.root/'.creation/identity.p12').is_file())
        self.assertFalse((self.root/'identity.json').exists())

    def test_format_repair_reuses_one_certificate_and_never_generates_a_second_key(self):
        der=b'synthetic public DER';pin=hashlib.sha1(der).hexdigest();calls=[]
        def run(args,**kwargs):
            calls.append(args)
            if 'req' in args:
                Path(args[args.index('-keyout')+1]).write_bytes(b'synthetic private fixture')
                Path(args[args.index('-out')+1]).write_bytes(b'synthetic certificate fixture')
            elif 'pkcs12' in args:Path(args[args.index('-out')+1]).write_bytes(b'synthetic encrypted envelope')
            return subprocess.CompletedProcess(args,0,'','')
        def public(args,**kwargs):return der if '-outform' in args else b'identical public key'
        with patch.object(setup.subprocess,'run',side_effect=run), \
             patch.object(setup.subprocess,'check_output',side_effect=public):
            with patch.object(setup,'import_identity',side_effect=setup.SetupRejected('native MAC error')):
                with self.assertRaises(setup.SetupRejected):setup.create(self.root)
            with patch.object(setup,'import_identity'), \
                 patch.object(setup,'identities',side_effect=[set(),{pin.upper()},{pin.upper()}]):
                result=setup.resume_format(self.root)
        self.assertEqual(sum('req' in command for command in calls),1)
        self.assertEqual(result['certificate_sha1'],pin)
        self.assertFalse((self.root/'.creation').exists())
        self.assertFalse(result['trust_settings_changed'])
