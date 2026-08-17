package dev.androiptv.cast

import android.app.Activity
import android.os.Handler
import android.os.Looper
import androidx.mediarouter.media.MediaRouteSelector
import androidx.mediarouter.media.MediaRouter
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin
import com.google.android.gms.cast.CastMediaControlIntent
import com.google.android.gms.cast.MediaInfo
import com.google.android.gms.cast.MediaLoadRequestData
import com.google.android.gms.cast.MediaMetadata
import com.google.android.gms.cast.framework.CastContext
import com.google.android.gms.cast.framework.CastSession
import com.google.android.gms.cast.framework.SessionManagerListener
import com.google.android.gms.cast.framework.media.RemoteMediaClient

@InvokeArg
class LoadArgs {
    var url: String? = null
    var title: String? = null
    var contentType: String? = null
    var streamType: String? = null
}

/**
 * Native Chromecast support. Google's web-sender JS SDK does not work
 * inside the Android WebView, so discovery, session management and media
 * loading run here with the AndroidX Cast SDK, exposed to the UI via
 * `plugin:cast|*` invokes. The device fetches the stream straight from
 * the app's embedded LAN TV server (the proxy URL the UI hands us).
 */
@TauriPlugin
class CastPlugin(private val activity: Activity) :
    Plugin(activity),
    SessionManagerListener<CastSession> {

    private val selector: MediaRouteSelector = MediaRouteSelector.Builder()
        .addControlCategory(
            CastMediaControlIntent.categoryForCast(
                CastMediaControlIntent.DEFAULT_MEDIA_RECEIVER_APPLICATION_ID,
            ),
        )
        .build()

    private var router: MediaRouter? = null

    /** true between selectRoute() and the session actually starting */
    @Volatile private var connecting = false
    @Volatile private var connected = false

    private val routerCallback = object : MediaRouter.Callback() {
        override fun onRouteSelected(
            router: MediaRouter,
            route: MediaRouter.RouteInfo,
            reason: Int,
        ) {
            connecting = true
        }

        override fun onRouteUnselected(
            router: MediaRouter,
            route: MediaRouter.RouteInfo,
            reason: Int,
        ) {
            connected = false
            connecting = false
        }
    }

    override fun load(webView: android.webkit.WebView) {
        super.load(webView)
        val appContext = activity.applicationContext

        // Initializes the Cast SDK: registers the cast route provider
        // (options from CastOptionsProvider) and the session manager.
        val castContext = CastContext.getSharedInstance(appContext)
        castContext.sessionManager.addSessionManagerListener(this, CastSession::class.java)

        // CALLBACK_FLAG_REQUEST_DISCOVERY starts mDNS discovery.
        router = MediaRouter.getInstance(appContext)
        router?.addCallback(routerCallback, selector, MediaRouter.CALLBACK_FLAG_REQUEST_DISCOVERY)
    }

    @Command
    fun isAvailable(invoke: Invoke) {
        val ret = JSObject()
        val hasRoutes = router
            ?.getRoutes()
            ?.any { it.isSelectable && it.matchesSelector(selector) } == true
        ret.put("available", hasRoutes)
        invoke.resolve(ret)
    }

    @Command
    fun connect(invoke: Invoke) {
        val target = router
            ?.getRoutes()
            ?.firstOrNull { it.isSelectable && it.matchesSelector(selector) }
        if (target == null) {
            invoke.reject("no cast device found on this network")
            return
        }
        connecting = true
        router?.selectRoute(target)
        invoke.resolve(JSObject())
    }

    @Command
    fun load(invoke: Invoke) {
        val args = invoke.parseArgs(LoadArgs::class.java)
        val sessionManager = CastContext.getSharedInstance(activity.applicationContext)
            .sessionManager

        // The session starts asynchronously after selectRoute(); wait
        // (bounded) for it before loading media.
        Thread {
            var waited = 0
            var session = sessionManager.currentCastSession
            while (session == null && waited < 150) {
                Thread.sleep(100)
                waited++
                session = sessionManager.currentCastSession
            }
            if (session == null) {
                invoke.reject("cast session did not start (is the device awake?)")
                return@Thread
            }
            // Cast SDK calls must run on the main thread.
            Handler(Looper.getMainLooper()).post { loadMedia(session!!, args, invoke) }
        }.start()
    }

    private fun loadMedia(session: CastSession, args: LoadArgs, invoke: Invoke) {
        val client = session.remoteMediaClient
        if (client == null) {
            invoke.reject("cast device has no media channel")
            return
        }
        val metadata = MediaMetadata(MediaMetadata.MEDIA_TYPE_MOVIE).apply {
            putString(MediaMetadata.KEY_TITLE, args.title ?: "")
        }
        val mediaInfo = MediaInfo.Builder(args.url ?: "")
            .setContentType(args.contentType ?: "video/mp4")
            .setStreamType(
                if (args.streamType == "live") MediaInfo.STREAM_TYPE_LIVE
                else MediaInfo.STREAM_TYPE_BUFFERED,
            )
            .setMetadata(metadata)
            .build()
        val request = MediaLoadRequestData.Builder()
            .setMediaInfo(mediaInfo)
            .setAutoplay(true)
            .build()
        client.load(request, object : RemoteMediaClient.MediaChannelResultCallback() {
            override fun onResult(result: RemoteMediaClient.MediaChannelResult) {
                if (result.status.isSuccess) {
                    invoke.resolve(JSObject())
                } else {
                    invoke.reject("media load failed on the device: ${result.status.statusCode}")
                }
            }
        })
    }

    @Command
    fun disconnect(invoke: Invoke) {
        connected = false
        connecting = false
        CastContext.getSharedInstance(activity.applicationContext)
            .sessionManager
            .endCurrentSession(true)
        invoke.resolve(JSObject())
    }

    @Command
    fun state(invoke: Invoke) {
        val session = CastContext.getSharedInstance(activity.applicationContext)
            .sessionManager
            .currentCastSession
        val ret = JSObject()
        ret.put(
            "state",
            when {
                connected -> "connected"
                connecting -> "connecting"
                else -> "disconnected"
            },
        )
        ret.put("device", session?.castDevice?.friendlyName ?: "")
        invoke.resolve(ret)
    }

    // ---- SessionManagerListener ----

    override fun onSessionStarted(session: CastSession, sessionId: String) {
        connected = true
        connecting = false
    }

    override fun onSessionEnded(session: CastSession, error: Int) {
        connected = false
        connecting = false
    }

    override fun onSessionStartFailed(session: CastSession, error: Int) {
        connected = false
        connecting = false
    }

    override fun onSessionResumed(session: CastSession, sessionId: String) {
        connected = true
    }

    override fun onSessionSuspended(session: CastSession, reason: Int) {
        connected = false
    }
}
