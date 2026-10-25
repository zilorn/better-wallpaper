"""Explicit live A5 probe. Requires niri, PipeWire/PulseAudio, dbus-python and Qt.
Run: dbus-run-session -- python3 native/web-wallpaper/tests/protocol.py
Uses an owned null audio sink and isolated MPRIS peer; never edits saved config.
"""
import http.server
import json
import math
import os
import pathlib
import struct
import sys
import subprocess
import tempfile
import threading
import time
import urllib.parse
import urllib.request
import wave
import dbus

ROOT = pathlib.Path(__file__).resolve().parents[3]
PLAYER = ROOT/'target/web-wallpaper/better-wallpaper-web'
PROBE = ROOT/'target/debug/examples/web_protocol_probe'

def wait(description, predicate, timeout=15):
    deadline = time.monotonic()+timeout
    while time.monotonic()<deadline:
        value=predicate()
        if value: return value
        time.sleep(0.05)
    raise AssertionError('Timeout: '+description)

def pactl(*arguments):
    return subprocess.check_output(['pactl',*map(str,arguments)],text=True)

def main():
    peer=subprocess.Popen(['/usr/bin/python3',str(ROOT/'native/web-wallpaper/tests/mpris_fixture.py')],stdout=subprocess.PIPE,text=True)
    assert peer.stdout.readline().strip()=='READY'
    bus=dbus.SessionBus()
    remote=bus.get_object('org.mpris.MediaPlayer2.betterwallpaper_a5','/org/mpris/MediaPlayer2')
    change=remote.get_dbus_method('Change','org.betterwallpaper.Test')
    module=pactl('load-module','module-null-sink','sink_name=better_wallpaper_a5','rate=48000','channels=2').strip()
    events=[]
    config={}
    class Handler(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            path=urllib.parse.urlparse(self.path)
            if path.path=='/api/v1/plasma/config': body=json.dumps(config).encode()
            else:
                value=urllib.parse.parse_qs(path.query).get('event')
                if value: events.append((time.monotonic(),json.loads(value[0])))
                body=b'{}'
            self.send_response(200);self.send_header('Content-Type','application/json');self.end_headers();self.wfile.write(body)
        def do_POST(self):
            self.rfile.read(int(self.headers.get('Content-Length','0')))
            self.send_response(200);self.end_headers();self.wfile.write(b'{}')
        def log_message(self,*args): pass
    server=http.server.ThreadingHTTPServer(('127.0.0.1',0),Handler)
    thread=threading.Thread(target=server.serve_forever,daemon=True);thread.start()
    origin=f'http://127.0.0.1:{server.server_port}'
    summary=[]
    try:
        with tempfile.TemporaryDirectory(prefix='a5-protocol-') as temporary:
            tone=pathlib.Path(temporary)/'stereo.wav'
            # Distinct left/right frequencies prove channel separation through the monitor.
            with wave.open(str(tone),'wb') as output:
                output.setparams((2,2,48000,0,'NONE','not compressed'))
                output.writeframes(b''.join(struct.pack('<hh',int(12000*math.sin(2*math.pi*440*i/48000)),int(12000*math.sin(2*math.pi*2000*i/48000))) for i in range(48000*30)))
            for backend in (sys.argv[1:] or ['niri','plasma-qml']):
                events.clear()
                change('Playing','Owned A5 Track',7.25)
                probe=subprocess.Popen([str(PROBE),str(ROOT/'tests/fixtures/web-protocol')],stdin=subprocess.PIPE,stdout=subprocess.PIPE,text=True)
                initial=json.loads(probe.stdout.readline())
                initial['url']+='?report='+urllib.parse.quote(origin+'/report',safe='')
                config.clear();config.update(enabled=True,paused=False,muted=True,loop_playback=True,fill_mode='cover',wallpaper_type='web',media_path='Owned A5 probe',revision=1,web_url=initial['url'],web_protocol_url=initial['protocolUrl'],web_properties=initial['properties'],fps_limit=37)
                arguments=[str(PLAYER)]
                if backend=='plasma-qml': arguments+=['--test-plasma-qml',str(ROOT/'kde/org.better-wallpaper/contents/ui/main.qml')]
                log_path=pathlib.Path(temporary)/(backend+'.log')
                with log_path.open('w') as log:
                    process=subprocess.Popen(arguments,stdin=subprocess.PIPE,stdout=log,stderr=log,text=True,env=dict(os.environ,QML_DISABLE_DISK_CACHE="1"))
                    tone_process=None
                    try:
                        def send(message): process.stdin.write(json.dumps(message)+'\n');process.stdin.flush()
                        send(dict(initial,daemonUrl=origin))
                        def seen(kind, predicate=lambda value:True):
                            return [event for _,event in events if event['kind']==kind and predicate(event['data'])]
                        wait(backend+' page',lambda:seen('properties') or process.poll() is not None)
                        assert process.poll() is None,log_path.read_text()
                        assert seen('properties')[-1]['data']['caption']['value']=='Saved caption'
                        assert seen('properties')[-1]['data']['enabled']['value'] is False
                        assert seen('properties')[-1]['data']['speed']['value']==0.25
                        wait('asset URL delivery',lambda:seen('asset',lambda value:value==64))
                        wait('initial callback order',lambda:seen('tick',lambda value:value['order']==['properties','general','pause']))
                        wait('general FPS',lambda:seen('general',lambda value:value['fps']==37))
                        wait('MPRIS metadata',lambda:seen('media',lambda value:value['title']=='Owned A5 Track' and value['artist']=='Owned Artist'))
                        wait('MPRIS timeline seconds',lambda:seen('timeline',lambda value:value['position']==7.25 and value['duration']==123))
                        def sources():
                            return [item for item in json.loads(pactl('-f','json','list','source-outputs')) if item['properties'].get('media.name')=='Web wallpaper stereo spectrum']
                        source=wait('stereo monitor stream',lambda:sources())[-1]
                        pactl('move-source-output',source['index'],'better_wallpaper_a5.monitor')
                        tone_process=subprocess.Popen(['paplay','--device=better_wallpaper_a5',str(tone)])
                        samples=wait('distinct live stereo spectrum',lambda:seen('audio',lambda value:value['length']==128 and value['left']>0.1 and value['right']>0.1 and value['leftPeak']<value['rightPeak']))
                        change('Paused','Changed A5 Track',8.5)
                        wait('changed media delivered',lambda:seen('media',lambda value:value['title']=='Changed A5 Track'))
                        wait('media paused state',lambda:seen('playback',lambda value:value['state']!=value['playing']))
                        before=len(seen('audio'))
                        if backend=='niri': send({'paused':True})
                        else: config['paused']=True
                        wait('pause callback',lambda:seen('pause',lambda value:value is True))
                        time.sleep(1.2);frozen=len(events)
                        time.sleep(0.6);assert len(events)==frozen,'callbacks/timers continued while paused'
                        wait('capture released after lease',lambda:not sources(),timeout=4)
                        if backend=='niri': send({'paused':False})
                        else: config['paused']=False
                        wait('resumed callbacks',lambda:len(seen('audio'))>before,timeout=10)
                        # Removing the consumer must stop data sources; re-add loads a fresh document.
                        registrations=len(seen('registration'))
                        if backend=='niri': send({'outputs':['__a5_missing_output__']})
                        else: config['enabled']=False
                        wait('capture released on output disable',lambda:not sources(),timeout=6)
                        time.sleep(0.5);removed=len(events);time.sleep(0.5)
                        assert len(events)==removed,'events survived output/view removal'
                        restored_start=len(events)
                        if backend=='niri': send({'outputs':[]})
                        else: config.update(enabled=True,revision=2)
                        wait('fresh registration after output restore',lambda:len(seen('registration'))>registrations,timeout=10)
                        wait('properties restored after reload',lambda:len(seen('properties'))>=2)
                        wait('capture restarted after output restore',lambda:sources())
                        if backend=='plasma-qml':
                            wait('restored MPRIS data',lambda:any(event['kind']=='media' and event['data']['title']=='Changed A5 Track' for _,event in events[restored_start:]))
                            before_loss=len(events)
                            peer.terminate();peer.wait(timeout=5)
                            wait('unavailable MPRIS source clears state',lambda:any(event['kind']=='tick' and event['data'].get('status',{}).get('mediaAvailable') is False for _,event in events[before_loss:]))
                            wait('removed media clears title',lambda:any(event['kind']=='media' and event['data']['title']=='' for _,event in events[before_loss:]))
                        process.stdin.close();process.wait(timeout=5);assert process.returncode==0
                        wait('capture released on view destruction',lambda:not sources(),timeout=4)
                        summary.append({'backend':backend,'audio_reports':len(seen('audio')),'stereo':samples[-1]['data'],'media_updates':len(seen('media')),'events':len(events),'result':'pass'})
                    finally:
                        if process.poll() is None: process.kill();process.wait()
                        if tone_process is not None: tone_process.terminate();tone_process.wait()
                        probe.stdin.close();probe.wait(timeout=5)
                        print(log_path.read_text())
    finally:
        server.shutdown();server.server_close();thread.join()
        pactl('unload-module',module)
        peer.terminate();peer.wait()
    print(json.dumps(summary,indent=2))
if __name__=='__main__': main()
