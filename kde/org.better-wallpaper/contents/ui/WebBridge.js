.pragma library
// Injected into Chromium's main world before the author's scripts.
var source = `(function () {
    'use strict';
    let initialized = false;
    let paused = true;
    let properties = Object.create(null);
    let general = null;
    let frame = null;
    const listeners = Object.create(null);
    const previous = Object.create(null);
    const copy = value => JSON.parse(JSON.stringify(value));
    function invoke(callback, value) {
        if (typeof callback !== 'function') return;
        try { callback(copy(value)); } catch (error) { console.error('Wallpaper callback failed', error); }
    }
    function event(name, value, force) {
        const encoded = JSON.stringify(value);
        if (!force && previous[name] === encoded) return;
        previous[name] = encoded;
        invoke(listeners[name], value);
    }
    function mediaEvents(force) {
        if (!frame || paused) return;
        event('MediaStatus', {enabled: !!frame.media_enabled}, force);
        const media = frame.media || {};
        event('MediaProperties', media.properties || {title:'',artist:'',subTitle:'',albumTitle:'',albumArtist:'',genres:'',contentType:'music'}, force);
        const states = window.wallpaperMediaIntegration;
        event('MediaThumbnail', {thumbnail:'',primaryColor:'#000000',secondaryColor:'#ffffff',tertiaryColor:'#ffffff',textColor:'#ffffff',highContrastColor:'#ffffff'}, force);
        event('MediaPlayback', {state: states['PLAYBACK_' + String(media.playback || 'stopped').toUpperCase()] ?? states.PLAYBACK_STOPPED}, force);
        event('MediaTimeline', media.timeline || {position:0,duration:0}, force);
    }
    const kinds = ['Audio', 'MediaStatus', 'MediaProperties', 'MediaPlayback', 'MediaTimeline', 'MediaThumbnail'];
    for (const kind of kinds) {
        window['wallpaperRegister' + kind + 'Listener'] = callback => {
            listeners[kind] = typeof callback === 'function' ? callback : null;
            if (initialized && !paused && kind !== 'Audio' && previous[kind]) invoke(listeners[kind], JSON.parse(previous[kind]));
        };
    }
    // Unsupported artwork receives an empty thumbnail during normal media dispatch.
    window.wallpaperMediaIntegration = Object.freeze({PLAYBACK_STOPPED:0,PLAYBACK_PLAYING:1,PLAYBACK_PAUSED:2,playback:Object.freeze({STOPPED:0,PLAYING:1,PAUSED:2})});
    window.betterWallpaperCapabilities = Object.freeze({stereoAudio:true,mediaProperties:true,mediaTimeline:true,mediaThumbnail:false,directoryProperties:false,filePropertyUrls:true});
    window.__betterWallpaper = {
        update(message) {
            const listener = window.wallpaperPropertyListener || {};
            if (message.properties) {
                const delta = Object.create(null);
                for (const key of Object.keys(message.properties)) {
                    if (!initialized || JSON.stringify(properties[key]) !== JSON.stringify(message.properties[key])) delta[key] = message.properties[key];
                }
                properties = copy(message.properties);
                if (Object.keys(delta).length) invoke(listener.applyUserProperties?.bind(listener), delta);
            }
            if (message.general && JSON.stringify(general) !== JSON.stringify(message.general)) {
                general = copy(message.general);
                invoke(listener.applyGeneralProperties?.bind(listener), general);
            }
            if (typeof message.paused === 'boolean') {
                const changed = !initialized || paused !== message.paused;
                paused = message.paused;
                if (changed) invoke(listener.setPaused?.bind(listener), paused);
            }
            initialized = true;
            if (message.frame) frame = message.frame;
            if (frame && !paused) {
                window.betterWallpaperStatus = Object.freeze({audioAvailable:Array.isArray(frame.audio),mediaAvailable:!!frame.media?.available});
                mediaEvents(false);
                if (message.frame && frame.audio_enabled && Array.isArray(frame.audio) && frame.audio.length === 128 && frame.audio.every(Number.isFinite)) invoke(listeners.Audio, frame.audio);
            }
        }
    };
    addEventListener('pagehide', () => {
        for (const key of Object.keys(listeners)) listeners[key] = null;
        frame = null;
        paused = true;
    }, {once:true});
})();`
