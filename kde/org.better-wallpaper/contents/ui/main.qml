// SPDX-License-Identifier: MIT

import QtQuick
import QtQuick.Window
import org.kde.plasma.plasmoid
import "BetterWallpaper"

WallpaperItem {
    id: root

    readonly property string daemonUrl: "http://127.0.0.1:43129"
    readonly property string outputName: Screen.name
    property bool wallpaperEnabled: false
    property bool wallpaperPaused: false
    property string framePath: ""
    property string configRevision: ""

    Component.onCompleted: {
        console.info("[Better Wallpaper] Plasma wallpaper instance created for output " + outputName)
        if (visible)
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
        }
    }

    function refreshConfig() {
        const request = new XMLHttpRequest()
        request.open("GET", daemonUrl + "/api/v1/plasma/config?output=" + encodeURIComponent(outputName))
        request.onreadystatechange = function() {
            if (request.readyState !== XMLHttpRequest.DONE)
                return
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
            frameView.fillMode = config.fill_mode
            if (wallpaperEnabled && (configRevision !== String(config.revision) || framePath === "")) {
                configRevision = String(config.revision)
                framePath = config.frame_path
                console.info("[Better Wallpaper] Plasma shared frame source configured: " + framePath)
            } else if (!wallpaperEnabled) {
                framePath = ""
            }
        }
        request.send()
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

    SharedFrameItem {
        id: frameView
        anchors.fill: parent
        source: root.wallpaperEnabled ? root.framePath : ""
        fillMode: "cover"
    }

    Timer {
        id: retryTimer
        interval: 3000
        repeat: false
        onTriggered: {
            console.warn("[Better Wallpaper] Plasma retrying daemon connection")
            root.refreshConfig()
        }
    }

    Timer {
        interval: 2000
        repeat: true
        running: root.visible
        onTriggered: root.refreshConfig()
    }
}
