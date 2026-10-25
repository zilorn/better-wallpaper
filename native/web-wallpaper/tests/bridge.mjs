import fs from 'node:fs';
import vm from 'node:vm';
import assert from 'node:assert/strict';
const text = fs.readFileSync('kde/org.better-wallpaper/contents/ui/WebBridge.js','utf8').replace('.pragma library','');
const library = vm.createContext({}); vm.runInContext(text, library);
function page() {
    const calls = [];
    const context = vm.createContext({console, calls, addEventListener:(name, callback)=>{context[name]=callback;}});
    context.window = context;
    vm.runInContext(library.source,context);
    vm.runInContext(`
        window.wallpaperPropertyListener = {
            applyUserProperties:p=>calls.push(['properties',p]),
            applyGeneralProperties:p=>calls.push(['general',p]),
            setPaused:p=>calls.push(['pause',p])};
        wallpaperRegisterAudioListener(a=>calls.push(['audio',a]));
        wallpaperRegisterMediaStatusListener(m=>calls.push(['status',m]));
        wallpaperRegisterMediaPropertiesListener(m=>calls.push(['media',m]));
        wallpaperRegisterMediaPlaybackListener(m=>calls.push(['playback',m]));
        wallpaperRegisterMediaTimelineListener(m=>calls.push(['timeline',m]));`,context);
    return {context,calls,update:message=>context.__betterWallpaper.update(message)};
}
const {context,calls,update}=page();
assert.equal(calls.length,0);
const frame={audio_enabled:true,media_enabled:true,audio:Array(128).fill(0.4),media:{available:true,properties:{title:'Real DBus title',artist:'Artist'},playback:'playing',timeline:{position:1,duration:10}}};
update({properties:{enabled:{value:false},speed:{value:0.25}},general:{fps:37},paused:false,frame});
assert.deepEqual(calls.map(c=>c[0]),['properties','general','pause','status','media','playback','timeline','audio']);
assert.equal(calls[0][1].enabled.value,false);
assert.equal(calls[0][1].speed.value,0.25);
assert.equal(calls[5][1].state,context.wallpaperMediaIntegration.PLAYBACK_PLAYING);
update({frame});
assert.equal(calls.filter(c=>c[0]==='media').length,1);
assert.equal(calls.filter(c=>c[0]==='audio').length,2);
update({properties:{enabled:{value:true},speed:{value:0.25}}});
assert.deepEqual(Object.keys(calls.findLast(c=>c[0]==='properties')[1]),['enabled']);
update({paused:true}); const frozen=calls.length; update({frame}); assert.equal(calls.length,frozen);
update({paused:false,frame:{...frame,media:{...frame.media,playback:'paused'}}});
assert.equal(calls.findLast(c=>c[0]==='playback')[1].state,context.wallpaperMediaIntegration.PLAYBACK_PAUSED);
let replacements=0; context.wallpaperRegisterMediaPropertiesListener(()=>replacements++);
assert.equal(replacements,1); context.wallpaperRegisterMediaPropertiesListener(null);
update({frame:{...frame,media:{...frame.media,properties:{title:'Changed'}}}}); assert.equal(replacements,1);
context.wallpaperRegisterAudioListener(()=>{throw Error('expected isolated callback failure');});
assert.doesNotThrow(()=>update({frame}));
context.pagehide(); const destroyed=calls.length; update({frame}); assert.equal(calls.length,destroyed);
const reloaded=page(); assert.equal(reloaded.calls.length,0);
assert.equal(context.betterWallpaperCapabilities.mediaThumbnail,false);
console.log('PASS: registration, typed properties, order, deltas, change-only media, stereo shape, constants, pause/resume, replacement, unsubscribe, exception isolation, teardown and fresh documents');
