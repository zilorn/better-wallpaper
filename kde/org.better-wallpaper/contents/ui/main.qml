// SPDX-License-Identifier: MIT

import QtQuick
import QtQuick.Window
import QtMultimedia
import QtCore
import org.kde.plasma.plasmoid

WallpaperItem {
    id: root

    property string daemonUrl: ""
    readonly property string outputName: Screen.name
    property bool wallpaperEnabled: false
    property bool wallpaperPaused: false
    property string mediaUrl: ""
    property string configRevision: ""

    Component.onCompleted: {
        console.info("[Better Wallpaper] Plasma wallpaper instance created for output " + outputName)
        discoverDaemon()
        if (visible && daemonUrl !== "")
            refreshConfig()
    }
    Component.onDestruction: console.info("[Better Wallpaper] Plasma wallpaper instance destroyed")
    onVisibleChanged: {
        if (visible) {
            console.info("[Better Wallpaper] Plasma wallpaper instance became visible")
            refreshConfig()
        } else {
            console.info("[Better Wallpaper] Plasma wallpaper instance became hidden; pausing playback")
            retryTimer.stop()
            player.pause()
        }
    }

    function refreshConfig() {
        if (daemonUrl === "") {
            discoverDaemon()
            return
        }
        const request = new XMLHttpRequest()
        request.open("GET", daemonUrl + "/api/v1/plasma/config?output=" + encodeURIComponent(outputName))
        request.onreadystatechange = function() {
            if (request.readyState !== XMLHttpRequest.DONE)
                return
            if (request.status !== 200) {
                console.warn("[Better Wallpaper] Plasma config request failed with status " + request.status)
                wallpaperEnabled = false
                player.stop()
                return
            }
            const config = JSON.parse(request.responseText)
            sendHeartbeat()
            wallpaperEnabled = config.enabled
            wallpaperPaused = config.paused
            videoOutput.fillMode = config.fill_mode === "contain"
                ? VideoOutput.PreserveAspectFit
                : config.fill_mode === "stretch" ? VideoOutput.Stretch : VideoOutput.PreserveAspectCrop
            player.audioOutput.muted = config.muted
            player.loops = config.loop_playback ? MediaPlayer.Infinite : 1
            const nextUrl = daemonUrl + config.media_url + "?revision=" + config.revision
            if (wallpaperEnabled && (configRevision !== String(config.revision) || player.source.toString() === "")) {
                configRevision = String(config.revision)
                mediaUrl = nextUrl
                player.source = mediaUrl
                if (wallpaperPaused)
                    player.pause()
                else
                    player.play()
            } else if (wallpaperEnabled && wallpaperPaused) {
                player.pause()
            } else if (wallpaperEnabled && player.playbackState !== MediaPlayer.PlayingState) {
                player.play()
            } else if (!wallpaperEnabled) {
                player.stop()
                player.source = ""
            }
        }
        request.send()
    }

    function discoverDaemon() {
        const runtimePath = StandardPaths.writableLocation(StandardPaths.RuntimeLocation)
        const homePath = StandardPaths.writableLocation(StandardPaths.HomeLocation)
        const candidates = [runtimePath + "/better-wallpaper/endpoint",
                            homePath + "/.better-wallpaper/better-wallpaper/endpoint"]
        for (let index = 0; index < candidates.length; ++index) {
            const request = new XMLHttpRequest()
            request.open("GET", "file://" + candidates[index], false)
            try {
                request.send()
                const endpoint = request.responseText.trim()
                if (endpoint.indexOf("http://127.0.0.1:") === 0) {
                    if (daemonUrl !== endpoint)
                        console.info("[Better Wallpaper] Discovered daemon endpoint " + endpoint)
                    daemonUrl = endpoint
                    return
                }
            } catch (error) {
                // The daemon may still be starting; the refresh timer will retry discovery.
            }
        }
        console.warn("[Better Wallpaper] Daemon endpoint discovery file is unavailable")
        daemonUrl = ""
    }

    function sendHeartbeat() {
        const request = new XMLHttpRequest()
        request.open("POST", daemonUrl + "/api/v1/plasma/heartbeat")
        request.setRequestHeader("Content-Type", "application/json")
        request.send(JSON.stringify({ "output": outputName }))
    }

    Rectangle {
        anchors.fill: parent
        color: "black"
    }

    VideoOutput {
        id: videoOutput
        anchors.fill: parent
        fillMode: VideoOutput.PreserveAspectCrop
    }

    MediaPlayer {
        id: player
        source: root.mediaUrl
        videoOutput: videoOutput
        loops: MediaPlayer.Infinite

        audioOutput: AudioOutput {
            muted: true
        }

        onErrorOccurred: (error, errorString) => {
            console.error("[Better Wallpaper] Plasma video playback failed: " + errorString)
            if (root.visible)
                retryTimer.restart()
        }
        onPlaybackStateChanged: console.info("[Better Wallpaper] Plasma playback state: " + playbackState)
    }

    Timer {
        id: retryTimer
        interval: 3000
        repeat: false
        onTriggered: {
            console.warn("[Better Wallpaper] Plasma retrying daemon connection")
            player.source = ""
            player.source = root.mediaUrl + "&retry=" + Date.now()
            player.play()
        }
    }

    Timer {
        interval: 2000
        repeat: true
        running: root.visible
        onTriggered: root.refreshConfig()
    }
}
