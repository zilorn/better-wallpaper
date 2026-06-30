// SPDX-License-Identifier: MIT

import QtQuick
import QtQuick.Window
import QtMultimedia
import QtWebEngine
import org.kde.plasma.plasmoid

WallpaperItem {
    id: root

    readonly property string daemonUrl: "http://127.0.0.1:43129"
    readonly property string outputName: Screen.name
    property bool wallpaperEnabled: false
    property bool wallpaperPaused: false
    property bool wallpaperMuted: true
    property bool loopPlayback: true
    property url mediaSource: ""
    property string wallpaperFillMode: "cover"
    property string configRevision: ""
    property string wallpaperType: "video"
    property url webSource: ""

    Component.onCompleted: {
        console.info("[Better Wallpaper] Plasma direct-render instance created for output " + outputName)
        if (visible) refreshConfig()
    }
    Component.onDestruction: console.info("[Better Wallpaper] Plasma direct-render instance destroyed")

    function syncPlayback() {
        if (wallpaperType === "video" && wallpaperEnabled && visible && !wallpaperPaused && mediaSource.toString() !== "") {
            if (mediaPlayer.playbackState !== MediaPlayer.PlayingState) mediaPlayer.play()
        } else if (mediaPlayer.playbackState === MediaPlayer.PlayingState) {
            mediaPlayer.pause()
        }
    }

    onVisibleChanged: {
        if (visible) refreshConfig()
        syncPlayback()
    }
    onWallpaperEnabledChanged: syncPlayback()
    onWallpaperPausedChanged: syncPlayback()
    onMediaSourceChanged: syncPlayback()

    function refreshConfig() {
        const request = new XMLHttpRequest()
        request.open("GET", daemonUrl + "/api/v1/plasma/config?output=" + encodeURIComponent(outputName))
        request.onreadystatechange = function() {
            if (request.readyState !== XMLHttpRequest.DONE) return
            if (request.status !== 200) {
                console.warn("[Better Wallpaper] Plasma config request failed with status " + request.status)
                wallpaperEnabled = false
                return
            }
            let config
            try {
                config = JSON.parse(request.responseText)
            } catch (error) {
                console.error("[Better Wallpaper] Plasma config response is invalid JSON: " + error)
                return
            }
            sendHeartbeat()
            wallpaperEnabled = config.enabled
            wallpaperPaused = config.paused
            wallpaperMuted = config.muted
            loopPlayback = config.loop_playback
            wallpaperFillMode = config.fill_mode
            wallpaperType = config.wallpaper_type || "video"
            if (wallpaperEnabled && config.media_path && configRevision !== String(config.revision)) {
                configRevision = String(config.revision)
                if (wallpaperType === "web") {
                    mediaSource = ""
                    webSource = daemonUrl + config.web_url + "?revision=" + config.revision
                    console.info("[Better Wallpaper] web wallpaper source configured: " + config.media_path)
                } else {
                    webSource = ""
                    mediaSource = daemonUrl + config.media_url + "?revision=" + config.revision
                    console.info("[Better Wallpaper] native media source configured: " + config.media_path)
                }
            } else if (!wallpaperEnabled || !config.media_path) {
                mediaSource = ""
                webSource = ""
            }
            syncPlayback()
        }
        request.send()
    }

    function sendHeartbeat() {
        const request = new XMLHttpRequest()
        request.open("POST", daemonUrl + "/api/v1/plasma/heartbeat")
        request.setRequestHeader("Content-Type", "application/json")
        request.send(JSON.stringify({ "output": outputName }))
    }

    Rectangle { anchors.fill: parent; color: "black" }

    VideoOutput {
        id: videoOutput
        anchors.fill: parent
        fillMode: root.wallpaperFillMode === "contain"
                  ? VideoOutput.PreserveAspectFit
                  : VideoOutput.PreserveAspectCrop
        visible: root.wallpaperType === "video"
    }

    WebEngineView {
        anchors.fill: parent
        visible: root.wallpaperType === "web" && root.wallpaperEnabled
        url: root.webSource
        audioMuted: root.wallpaperMuted || root.wallpaperPaused
        settings.localContentCanAccessRemoteUrls: true
        onLoadingChanged: (loadRequest) => {
            if (loadRequest.status === WebEngineView.LoadFailedStatus)
                console.error("[Better Wallpaper] web wallpaper load failed: " + loadRequest.errorString)
        }
    }

    AudioOutput {
        id: wallpaperAudio
        muted: root.wallpaperMuted
    }

    MediaPlayer {
        id: mediaPlayer
        source: root.mediaSource
        audioOutput: wallpaperAudio
        videoOutput: videoOutput
        loops: root.loopPlayback ? MediaPlayer.Infinite : 1
        onErrorOccurred: (error, errorString) =>
            console.error("[Better Wallpaper] native media playback failed: " + errorString)
    }

    Timer {
        interval: 2000
        repeat: true
        running: root.visible
        onTriggered: root.refreshConfig()
    }
}
