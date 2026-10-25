// SPDX-License-Identifier: MIT
import QtQuick
import QtWebEngine
import "WebBridge.js" as Bridge

Item {
    id: root
    property url sourceUrl: ""
    property url protocolUrl: ""
    property string propertiesJson: "{}"
    property bool paused: false
    property bool muted: true
    property int fpsLimit: 60
    property bool captured: false
    property var capturedResult: null
    property bool loaded: false
    property var pendingRequest: null
    property int generation: 0

    function update(message) {
        if (loaded) browser.runJavaScript("window.__betterWallpaper?.update(" + JSON.stringify(message) + ")")
    }
    function abortRequest() {
        if (pendingRequest) {
            pendingRequest.onreadystatechange = null
            pendingRequest.abort()
            pendingRequest = null
        }
    }
    function poll() {
        if (pendingRequest || !loaded || paused || !visible || protocolUrl.toString() === "") return
        const request = new XMLHttpRequest()
        const current = generation
        pendingRequest = request
        request.open("GET", protocolUrl)
        request.onreadystatechange = function() {
            if (request.readyState !== XMLHttpRequest.DONE) return
            if (root.pendingRequest === request) root.pendingRequest = null
            if (current !== root.generation || root.paused || !root.visible || request.status !== 200) return
            try { root.update({frame: JSON.parse(request.responseText)}) }
            catch (error) { console.warn("Invalid wallpaper protocol data: " + error) }
        }
        request.send()
    }
    function syncPause() {
        abortRequest()
        if (!paused && visible) {
            browser.lifecycleState = WebEngineView.LifecycleState.Active
            captured = false
            snapshot.source = ""
            capturedResult = null
            update({paused:false})
            poll()
        } else if (loaded && !captured) {
            update({paused:true})
            const current = generation
            browser.grabToImage(function(result) {
                if ((!root.paused && root.visible) || current !== root.generation) return
                root.capturedResult = result
                snapshot.source = result.url
                root.captured = true
                browser.lifecycleState = WebEngineView.LifecycleState.Frozen
            })
        }
    }
    onPausedChanged: syncPause()
    onVisibleChanged: syncPause()
    onPropertiesJsonChanged: { if (loaded) update({properties:JSON.parse(propertiesJson)}) }
    onFpsLimitChanged: update({general:{fps:fpsLimit}})
    function navigate() {
        generation++
        loaded = false
        abortRequest()
        if (!browser) return
        browser.lifecycleState = WebEngineView.LifecycleState.Active
        captured = false
        capturedResult = null
        snapshot.source = ""
        const target = sourceUrl.toString() === "" ? "about:blank" : sourceUrl.toString()
        if (target === "about:blank" && (browser.url.toString() === "" || browser.url.toString() === "about:blank")) return
        if (browser.url.toString() !== target) browser.url = target
        else if (target !== "about:blank") browser.reload()
    }
    onSourceUrlChanged: Qt.callLater(navigate)
    Component.onCompleted: Qt.callLater(navigate)
    Component.onDestruction: abortRequest()

    Rectangle { anchors.fill: parent; color: "black" }
    WebEngineView {
        id: browser
        anchors.fill: parent
        visible: !root.captured
        url: ""
        audioMuted: root.muted || root.paused || !root.visible
        backgroundColor: "black"
        settings.localContentCanAccessFileUrls: false
        settings.localContentCanAccessRemoteUrls: false
        userScripts.collection: [{
            name: "wallpaper-engine-compatibility",
            injectionPoint: WebEngineScript.DocumentCreation,
            worldId: WebEngineScript.MainWorld,
            runsOnSubFrames: false,
            sourceCode: Bridge.source
        }]
        onLifecycleStateChanged: console.info("Web wallpaper lifecycle state: " + lifecycleState)
        onLoadingChanged: (request) => {
            if (request.status === WebEngineView.LoadStartedStatus) {
                root.loaded = false
                root.generation++
                root.abortRequest()
            } else if (request.status === WebEngineView.LoadFailedStatus) {
                console.error("Web wallpaper load failed: " + request.errorString)
            } else if (request.status === WebEngineView.LoadSucceededStatus) {
                if (request.url.toString() === "about:blank" || request.url.toString() === "") {
                    if (root.sourceUrl.toString() !== "") Qt.callLater(root.navigate)
                    return
                }
                if (root.sourceUrl.toString() === "") return
                root.loaded = true
                console.info("Web wallpaper loaded: " + browser.url)
                root.update({properties:JSON.parse(root.propertiesJson), general:{fps:root.fpsLimit}, paused:root.paused || !root.visible})
                root.syncPause()
            }
        }
        onRenderProcessTerminated: (status, code) => {
            console.error("Web wallpaper renderer terminated: " + status + ", exit " + code)
            root.loaded = false
            root.abortRequest()
            reloadTimer.start()
        }
        onNewWindowRequested: (request) => console.warn("Web wallpaper popup ignored")
        onFeaturePermissionRequested: (origin, feature) => grantFeaturePermission(origin, feature, false)
        onNavigationRequested: (request) => {
            // Author resources may use network access; top-level pages stay on the private origin.
            const source = root.sourceUrl.toString().match(/^https?:\/\/[^/]+/)
            if (request.url.toString() !== "about:blank" && request.isMainFrame && source && !request.url.toString().startsWith(source[0] + "/")) request.action = WebEngineNavigationRequest.IgnoreRequest
        }
    }
    Image { id: snapshot; anchors.fill: parent; visible: root.captured }
    Timer { interval:33; repeat:true; running:root.loaded && !root.paused && root.visible; onTriggered:root.poll() }
    Timer {
        id: reloadTimer
        interval:1000
        onTriggered: {
            browser.lifecycleState = WebEngineView.LifecycleState.Active
            root.captured = false
            browser.reload()
        }
    }
}
