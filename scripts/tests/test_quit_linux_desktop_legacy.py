"""Mock product/window lifecycle plus an owned native SIGINT child fixture."""
import copy
import os
from pathlib import Path
import signal
import subprocess
import sys
import unittest
from unittest.mock import patch

import test_linux_desktop_profile_launch as launch_fixture
import quit_linux_desktop_legacy as quit
import upgrade_linux_desktop as up

class LegacyQuitTests(unittest.TestCase):
    def setUp(self):
        launch_fixture.ProfileLaunchTests.setUp(self)
        p=self.c.plan();self.c.apply(p,up.gate.digest(up.gate.canonical(p)))
        self.is_stopped=False;self.known={101,102};self.events=[];self.failure=None
        self.control.query=lambda *_:{'build_identity':{'source':{'commit':quit.SOURCE}}}
        self.control.frontend_busy=lambda:102 in self.known
        self.control.owners=lambda _:set(self.known)
        owner=self
        class Observer:
            def __init__(self,identity):self.i=identity;owner.events.append(('observe',identity['pid']))
            def verify(self):owner.events.append(('verify',self.i['pid']))
            def interrupt(self):owner.events.append(('interrupt',self.i['pid']))
            def wait(self):
                pid=self.i['pid'];owner.events.append(('wait',pid))
                if owner.failure==pid:raise TimeoutError('synthetic unknown normal exit')
                owner.known.discard(pid)
                if pid==101:owner.is_stopped=True
            def release(self):owner.events.append(('release',self.i['pid']))
        for obj,name,value in [(quit,'SHA',self.old_sha),(quit,'Observer',Observer),
            (quit,'target',lambda pid,exe,sha:{'pid':pid,'uid':os.getuid(),'start_ticks':str(pid),'executable':str(exe),'executable_sha256':sha}),
            (quit,'window',lambda pid:{'pid':pid,'id':'{3ce50827-6483-47a7-a87b-c870902a5d43}'}),
            (quit,'close',lambda w:self.events.append(('normal-window-close',w['pid'])))]:
            m=patch.object(obj,name,value);m.start();self.addCleanup(m.stop)
        self.q=quit.Quit(self.home,self.control)
    def plan(self):
        p=self.q.plan();return p,up.gate.digest(up.gate.canonical(p))
    def intact(self):
        self.assertEqual(up.profile_pin(self.paths['profile']),self.before_profile)
        self.assertEqual((self.paths['work']/'state.json').read_bytes(),self.original)
        self.assertFalse(self.up.transaction.exists())
    def test_exact_close_and_interrupt_once_then_independent_stopped_locks(self):
        p,d=self.plan();r=self.q.apply(p,d)
        self.assertEqual(r['phase'],'completed');self.assertFalse(r['idle_only_guarantee'])
        self.assertEqual(self.events.count(('normal-window-close',102)),1);self.assertEqual(self.events.count(('interrupt',101)),1)
        self.assertLess(self.events.index(('observe',101)),self.events.index(('normal-window-close',102)))
        self.assertLess(self.events.index(('wait',102)),self.events.index(('interrupt',101)));self.intact()
    def test_gui_timeout_never_sends_background_signal_or_replays_close(self):
        p,d=self.plan();self.failure=102
        with self.assertRaises(TimeoutError):self.q.apply(p,d)
        self.assertEqual(self.q.status()['phase'],'gui-close-intent')
        with self.assertRaises(ValueError):self.q.apply(p,d)
        self.assertEqual(self.events.count(('normal-window-close',102)),1);self.assertNotIn(('interrupt',101),self.events);self.intact()
    def test_background_timeout_never_repeats_signal(self):
        p,d=self.plan();self.failure=101
        with self.assertRaises(TimeoutError):self.q.apply(p,d)
        self.assertEqual(self.q.status()['phase'],'background-interrupt-intent')
        with self.assertRaises(ValueError):self.q.apply(p,d)
        self.assertEqual(self.events.count(('interrupt',101)),1);self.intact()
    def test_wrong_plan_owner_or_window_refuses_before_exit_intent(self):
        p,d=self.plan()
        with self.assertRaises(ValueError):self.q.apply(p,'0'*64)
        with patch.object(quit,'window',return_value={'pid':102,'id':'different'}),self.assertRaises(ValueError):self.q.apply(p,d)
        self.assertFalse(self.q.record.exists());self.assertEqual(self.events,[]);self.intact()
    def test_unreviewed_binary_or_source_refuse_before_window_queries(self):
        with patch.object(quit,'SHA','0'*64),patch.object(quit,'window') as w,self.assertRaises(ValueError):self.plan()
        w.assert_not_called()
        self.control.query=lambda *_:{'build_identity':{'source':{'commit':'different'}}}
        with patch.object(quit,'window') as w,self.assertRaises(ValueError):self.plan()
        w.assert_not_called();self.intact()
    def test_changed_original_data_or_replacement_background_blocks_signal(self):
        p,d=self.plan()
        self.control.owners=lambda _:{101,909}
        with self.assertRaises(ValueError):self.q.apply(p,d)
        self.assertNotIn(('interrupt',101),self.events);self.intact()
    def test_visible_active_work_refusal_propagates_without_window_close(self):
        self.control.observe=lambda _:(_ for _ in ()).throw(ValueError('synthetic active session/transfer'))
        with self.assertRaises(ValueError):self.plan()
        self.assertEqual(self.events,[]);self.intact()
    def test_observer_registration_failure_precedes_any_close(self):
        p,d=self.plan()
        with patch.object(quit,'Observer',side_effect=PermissionError('synthetic pidfd denial')),self.assertRaises(PermissionError):self.q.apply(p,d)
        self.assertFalse(self.q.record.exists());self.assertEqual(self.events,[]);self.intact()
    def test_opaque_configuration_change_after_close_blocks_interrupt(self):
        p,d=self.plan()
        def changed(w):
            self.events.append(('normal-window-close',w['pid']))
            self.environment.write_bytes(b'changed-synthetic-environment')
        with patch.object(quit,'close',changed),self.assertRaises(ValueError):self.q.apply(p,d)
        self.assertNotIn(('interrupt',101),self.events);self.assertTrue(self.environment.exists());self.intact()
    def test_loaded_policy_change_before_close_blocks_both_requests(self):
        p,d=self.plan();original=self.q.configuration;calls=0
        def changed():
            nonlocal calls
            calls+=1;c=original()
            if calls>=2:c=copy.deepcopy(c);c['policy']['changed-synthetic-policy']=True
            return c
        self.q.configuration=changed
        with self.assertRaises(ValueError):self.q.apply(p,d)
        self.assertNotIn(('normal-window-close',102),self.events);self.assertNotIn(('interrupt',101),self.events);self.intact()

class NativeInterruptTests(unittest.TestCase):
    @unittest.skipUnless(sys.platform.startswith('linux') and hasattr(os,'pidfd_open') and hasattr(signal,'pidfd_send_signal'),'Native Linux pidfd fixture')
    def test_owned_child_handles_one_sigint_without_killing_an_unrelated_process(self):
        child=subprocess.Popen([sys.executable,'-c','import signal,time,sys;signal.signal(signal.SIGINT,lambda *_:sys.exit(0));print("ready",flush=True);time.sleep(20)'],stdout=subprocess.PIPE,text=True)
        try:
            self.assertEqual(child.stdout.readline().strip(),'ready');exe=Path(sys.executable).resolve()
            identity=quit.target(child.pid,exe,up.gate.hash_file(exe));observer=quit.Observer(identity)
            try:observer.interrupt();observer.wait()
            finally:observer.release()
            self.assertEqual(child.wait(timeout=5),0)
        finally:
            if child.poll() is None:child.kill();child.wait()
            child.stdout.close()

if __name__=='__main__':unittest.main()
