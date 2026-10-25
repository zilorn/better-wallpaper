// SPDX-License-Identifier: MIT

import QtQuick
import QtQuick.Window
import QtMultimedia
import QtWebEngine
import org.kde.plasma.plasmoid

WallpaperItem {
    id: root

    property string daemonUrl: "http://127.0.0.1:43129"
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
    property var webProperties: ({})
    property url webProtocolUrl: ""
    property int webFpsLimit: 60
    property bool sceneRenderingAvailable: false

    Component.onCompleted: {
        console.info("[Better Wallpaper] Plasma wallpaper instance created for output " + outputName)
        if (visible) refreshConfig()
    }
    Component.onDestruction: console.info("[Better Wallpaper] Plasma wallpaper instance destroyed")

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
            webProperties = config.web_properties || ({})
            webProtocolUrl = config.web_protocol_url || ""
            webFpsLimit = config.fps_limit || 60
            sceneRenderingAvailable = config.scene_rendering_available === true
            if (wallpaperEnabled && config.media_path && configRevision !== String(config.revision)) {
                configRevision = String(config.revision)
                if (wallpaperType === "web") {
                    mediaSource = ""
                    webSource = config.web_url ? config.web_url + (config.web_url.indexOf("?") >= 0 ? "&" : "?") + "revision=" + config.revision : ""
                    console.info("[Better Wallpaper] web wallpaper source configured: " + config.media_path)
                } else if (wallpaperType === "video") {
                    webSource = ""
                    mediaSource = daemonUrl + config.media_url + "?revision=" + config.revision
                    console.info("[Better Wallpaper] HTTP media source configured: " + mediaSource)
                } else {
                    webSource = ""
                    mediaSource = ""
                    console.warn("[Better Wallpaper] scene rendering is not available in this build")
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

    WebSurface {
        id: webView
        anchors.fill: parent
        visible: root.wallpaperType === "web" && root.wallpaperEnabled
        sourceUrl: root.webSource
        protocolUrl: root.webProtocolUrl
        propertiesJson: JSON.stringify(root.webProperties)
        fpsLimit: root.webFpsLimit
        paused: root.wallpaperPaused || !root.visible || !root.wallpaperEnabled
        muted: root.wallpaperMuted
    }

    Column {
        anchors.centerIn: parent
        spacing: 8
        visible: root.wallpaperType === "scene" && root.wallpaperEnabled

        Text {
            anchors.horizontalCenter: parent.horizontalCenter
            color: "white"
            font.pixelSize: 20
            text: "Better Wallpaper Scene"
        }
        Text {
            anchors.horizontalCenter: parent.horizontalCenter
            color: "#b8b8b8"
            text: root.sceneRenderingAvailable
                  ? "Loading scene…"
                  : "Scene texture rendering is not available yet"
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
            console.error("[Better Wallpaper] Qt Multimedia playback failed: " + errorString)
    }

    Timer {
        interval: 2000
        repeat: true
        running: root.visible
        onTriggered: root.refreshConfig()
    }
}
