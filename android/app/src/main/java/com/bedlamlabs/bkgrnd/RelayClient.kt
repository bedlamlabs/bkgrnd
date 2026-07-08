package com.bedlamlabs.bkgrnd

import android.content.Context
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody.Companion.toRequestBody
import org.json.JSONArray
import org.json.JSONObject
import java.net.URLEncoder

/** A stream/track from the relay library. */
data class Track(
    val url: String,
    val title: String,
    val channel: String,
    val thumbnail: String,
)

/**
 * Talks to the same relay the PWA/iOS use. Auth is per-user: [login] returns a
 * session token (relay change a274bee) which we store and send back as a
 * `Cookie: bkgrnd_session=…` header on every request. YouTube-only in v1.
 */
class RelayClient(context: Context) {
    private val base = "https://bkgrnd.bedl.am"
    private val http = OkHttpClient()
    private val prefs = context.getSharedPreferences("bkgrnd", Context.MODE_PRIVATE)

    var token: String?
        get() = prefs.getString("session_token", null)
        private set(value) = prefs.edit().putString("session_token", value).apply()

    val isLoggedIn: Boolean get() = token != null

    fun logout() = prefs.edit().remove("session_token").apply()

    /** Cookie header carrying the session token (native equivalent of the PWA cookie). */
    fun authHeaders(): Map<String, String> =
        token?.let { mapOf("Cookie" to "bkgrnd_session=$it") } ?: emptyMap()

    suspend fun login(username: String, password: String): Result<Unit> = withContext(Dispatchers.IO) {
        runCatching {
            val body = JSONObject().put("username", username).put("password", password)
                .toString().toRequestBody("application/json".toMediaType())
            val req = Request.Builder().url("$base/api/v1/pwa/login").post(body).build()
            http.newCall(req).execute().use { resp ->
                if (!resp.isSuccessful) error("login failed (${resp.code})")
                val json = JSONObject(resp.body!!.string())
                token = json.getString("token")
            }
        }
    }

    suspend fun fetchLibrary(): List<Track> = withContext(Dispatchers.IO) {
        val req = authedGet("$base/api/v1/playlists.json")
        http.newCall(req).execute().use { resp ->
            if (!resp.isSuccessful) return@withContext emptyList()
            parseLibrary(JSONObject(resp.body!!.string()))
        }
    }

    suspend fun search(query: String): List<Track> = withContext(Dispatchers.IO) {
        val q = URLEncoder.encode(query, "UTF-8")
        val req = authedGet("$base/api/v1/search?q=$q")
        http.newCall(req).execute().use { resp ->
            if (!resp.isSuccessful) return@withContext emptyList()
            parseTracks(JSONArray(resp.body!!.string()))
        }
    }

    /** The proxied stream URL ExoPlayer plays (send the Cookie via the data source). */
    fun streamUrl(sourceUrl: String): String {
        val u = URLEncoder.encode(sourceUrl, "UTF-8")
        return "$base/api/v1/stream?proxy=true&url=$u"
    }

    private fun authedGet(url: String): Request {
        val b = Request.Builder().url(url)
        authHeaders().forEach { (k, v) -> b.header(k, v) }
        return b.build()
    }

    private fun parseLibrary(doc: JSONObject): List<Track> {
        val out = ArrayList<Track>()
        val seen = HashSet<String>()  // dedup by YouTube video id (parity with other clients)
        val playlists = doc.optJSONArray("playlists") ?: return out
        for (i in 0 until playlists.length()) {
            val items = playlists.getJSONObject(i).optJSONArray("items") ?: continue
            for (t in parseTracks(items)) {
                val key = videoKey(t.url)
                if (t.url.isNotEmpty() && seen.add(key)) out.add(t)
            }
        }
        return out
    }

    private fun parseTracks(arr: JSONArray): List<Track> {
        val out = ArrayList<Track>(arr.length())
        for (i in 0 until arr.length()) {
            val o = arr.getJSONObject(i)
            val url = o.optString("url")
            out.add(
                Track(
                    url = url,
                    title = o.optString("title", url),
                    channel = o.optString("channel", ""),
                    thumbnail = o.optString("thumbnail", ""),
                )
            )
        }
        return out
    }

    private fun videoKey(url: String): String {
        val m = Regex("(?:v=|youtu\\.be/|/shorts/)([A-Za-z0-9_-]{11})").find(url)
        return m?.groupValues?.get(1) ?: url
    }
}
