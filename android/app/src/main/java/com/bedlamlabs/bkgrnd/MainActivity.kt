package com.bedlamlabs.bkgrnd

import android.content.ComponentName
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import androidx.media3.common.MediaItem
import androidx.media3.common.MediaMetadata
import androidx.media3.session.MediaController
import androidx.media3.session.SessionToken
import coil.compose.AsyncImage
import com.google.common.util.concurrent.MoreExecutors
import kotlinx.coroutines.launch

/**
 * Starter UI: login → library list → tap to play through [PlaybackService].
 * Phone-only v1. UI is intentionally minimal — flesh out (player screen, search,
 * bookmark) in Android Studio; the relay client + playback service are the core.
 */
class MainActivity : ComponentActivity() {
    private var controller: MediaController? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val token = SessionToken(this, ComponentName(this, PlaybackService::class.java))
        val future = MediaController.Builder(this, token).buildAsync()
        future.addListener({ controller = future.get() }, MoreExecutors.directExecutor())

        setContent { MaterialTheme(colorScheme = darkColorScheme()) { App() } }
    }

    override fun onDestroy() {
        controller?.release()
        super.onDestroy()
    }

    private fun play(track: Track, relay: RelayClient) {
        val item = MediaItem.Builder()
            .setUri(relay.streamUrl(track.url))
            .setMediaMetadata(
                MediaMetadata.Builder()
                    .setTitle(track.title)
                    .setArtist(track.channel)
                    .build()
            )
            .build()
        controller?.apply {
            setMediaItem(item)
            prepare()
            playWhenReady = true
        }
    }

    @Composable
    private fun App() {
        val relay = remember { RelayClient(this) }
        var loggedIn by remember { mutableStateOf(relay.isLoggedIn) }
        Surface(Modifier.fillMaxSize()) {
            if (loggedIn) LibraryScreen(relay) else LoginScreen(relay) { loggedIn = true }
        }
    }

    @Composable
    private fun LoginScreen(relay: RelayClient, onDone: () -> Unit) {
        val scope = rememberCoroutineScope()
        var user by remember { mutableStateOf("") }
        var pass by remember { mutableStateOf("") }
        var error by remember { mutableStateOf<String?>(null) }
        var busy by remember { mutableStateOf(false) }
        Column(
            Modifier.fillMaxSize().padding(24.dp),
            verticalArrangement = Arrangement.Center
        ) {
            Text("bkgrnd", style = MaterialTheme.typography.headlineMedium)
            Spacer(Modifier.height(16.dp))
            OutlinedTextField(user, { user = it }, label = { Text("Username") }, singleLine = true)
            Spacer(Modifier.height(8.dp))
            OutlinedTextField(pass, { pass = it }, label = { Text("Password") }, singleLine = true)
            error?.let { Spacer(Modifier.height(8.dp)); Text(it, color = MaterialTheme.colorScheme.error) }
            Spacer(Modifier.height(16.dp))
            Button(
                enabled = !busy,
                onClick = {
                    busy = true; error = null
                    scope.launch {
                        relay.login(user.trim(), pass).onSuccess { onDone() }
                            .onFailure { error = "Invalid username or password" }
                        busy = false
                    }
                }
            ) { Text("Sign in") }
        }
    }

    @Composable
    private fun LibraryScreen(relay: RelayClient) {
        var tracks by remember { mutableStateOf<List<Track>>(emptyList()) }
        LaunchedEffect(Unit) { tracks = relay.fetchLibrary() }
        LazyColumn(Modifier.fillMaxSize().padding(top = 32.dp)) {
            item {
                Text("Streams", Modifier.padding(16.dp), style = MaterialTheme.typography.titleLarge)
            }
            items(tracks) { track ->
                Row(
                    Modifier.fillMaxWidth().clickable { play(track, relay) }.padding(12.dp)
                ) {
                    AsyncImage(model = track.thumbnail, contentDescription = null, modifier = Modifier.size(56.dp))
                    Spacer(Modifier.width(12.dp))
                    Column {
                        Text(track.title, style = MaterialTheme.typography.bodyLarge, maxLines = 2)
                        if (track.channel.isNotEmpty()) {
                            Text(track.channel, style = MaterialTheme.typography.bodySmall)
                        }
                    }
                }
            }
        }
    }
}
