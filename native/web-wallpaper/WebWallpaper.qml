// SPDX-License-Identifier: MIT
import QtQuick
import QtWebEngine

Item {
    id: root
    property url sourceUrl: ""
    property string propertiesJson: "{}"
    property bool paused: false
    property bool muted: true
    property int fpsLimit: 60
    property bool captured: false
    property var capturedResult: null

    function syncPause() {
        if (!paused) {
            browser.lifecycleState = WebEngineView.LifecycleState.Active
            captured = false
            snapshot.source = ""
            capturedResult = null
            browser.runJavaScript("window.wallpaperPropertyListener?.setPaused?.(false)")
        } else if (!browser.loading && browser.url.toString() !== "") {
            browser.runJavaScript("window.wallpaperPropertyListener?.setPaused?.(true)")
            browser.grabToImage(function(result) {
                if (!root.paused) return
                root.capturedResult = result
                snapshot.source = result.url
                root.captured = true
                browser.lifecycleState = WebEngineView.LifecycleState.Frozen
            })
        }
    }
    onPausedChanged: syncPause()

    Rectangle { anchors.fill: parent; color: "black" }
    WebEngineView {
        id: browser
        anchors.fill: parent
        visible: !root.captured
        url: root.sourceUrl
        audioMuted: root.muted || root.paused
        onLifecycleStateChanged: console.info("Web wallpaper lifecycle state: " + lifecycleState)
        backgroundColor: "black"
        settings.localContentCanAccessFileUrls: false
        settings.localContentCanAccessRemoteUrls: false
        userScripts.collection: [{
            name: "wallpaper-engine-compatibility",
            injectionPoint: WebEngineScript.DocumentCreation,
            worldId: WebEngineScript.MainWorld,
            sourceCode: "window.wallpaperRegisterAudioListener = function() {}; window.wallpaperRegisterMediaPropertiesListener = function() {};"
        }]
        onLoadingChanged: (request) => {
            if (request.status === WebEngineView.LoadFailedStatus) {
                console.error("Web wallpaper load failed: " + request.errorString)
            } else if (request.status === WebEngineView.LoadSucceededStatus) {
                console.info("Web wallpaper loaded: " + browser.url)
                browser.runJavaScript("window.wallpaperPropertyListener?.applyUserProperties?.(" + root.propertiesJson + "); window.wallpaperPropertyListener?.applyGeneralProperties?.({fps:" + root.fpsLimit + "});")
                root.syncPause()
            }
        }
        onRenderProcessTerminated: (status, code) => {
            console.error("Web wallpaper renderer terminated: " + status + ", exit " + code)
            reloadTimer.start()
        }
        onNewWindowRequested: (request) => console.warn("Web wallpaper popup ignored")
        onFeaturePermissionRequested: (origin, feature) => grantFeaturePermission(origin, feature, false)
    }
    Image { id: snapshot; anchors.fill: parent; visible: root.captured }
    Timer {
        id: reloadTimer
        interval: 1000
        onTriggered: {
            browser.lifecycleState = WebEngineView.LifecycleState.Active
            root.captured = false
            browser.reload()
        }
    }
}
