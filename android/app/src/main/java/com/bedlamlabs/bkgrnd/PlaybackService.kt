package com.bedlamlabs.bkgrnd

import androidx.media3.common.util.UnstableApi
import androidx.media3.datasource.DefaultHttpDataSource
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.exoplayer.source.DefaultMediaSourceFactory
import androidx.media3.session.MediaSession
import androidx.media3.session.MediaSessionService

/**
 * Media3 playback service. Gives lock-screen / notification transport controls
 * for free via [MediaSession]. Streams the relay's proxied `/api/v1/stream` URL,
 * injecting the session cookie on the HTTP fetch so it authenticates.
 *
 * Phone-only for v1. Swapping [MediaSessionService] for `MediaLibraryService`
 * and exposing a browse tree is what later turns this into an Android Auto app.
 */
@UnstableApi
class PlaybackService : MediaSessionService() {
    private var mediaSession: MediaSession? = null

    override fun onCreate() {
        super.onCreate()
        val relay = RelayClient(this)
        val httpFactory = DefaultHttpDataSource.Factory().apply {
            relay.authHeaders().takeIf { it.isNotEmpty() }?.let { setDefaultRequestProperties(it) }
        }
        val player = ExoPlayer.Builder(this)
            .setMediaSourceFactory(DefaultMediaSourceFactory(httpFactory))
            .setHandleAudioBecomingNoisy(true)
            .build()
        mediaSession = MediaSession.Builder(this, player).build()
    }

    override fun onGetSession(controllerInfo: MediaSession.ControllerInfo): MediaSession? =
        mediaSession

    override fun onDestroy() {
        mediaSession?.run {
            player.release()
            release()
        }
        mediaSession = null
        super.onDestroy()
    }
}
