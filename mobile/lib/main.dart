import 'dart:convert';

import 'package:cached_network_image/cached_network_image.dart';
import 'package:flutter/material.dart';
import 'package:http/http.dart' as http;
import 'package:just_audio/just_audio.dart';
import 'package:just_audio_background/just_audio_background.dart';
import 'package:shared_preferences/shared_preferences.dart';

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  await JustAudioBackground.init(
    androidNotificationChannelId: 'com.bedlamlabs.bkgrnd.audio',
    androidNotificationChannelName: 'bkgrnd playback',
    androidNotificationOngoing: true,
  );
  runApp(const BkgrndApp());
}

class Track {
  final String url, title, channel, thumbnail;
  Track({required this.url, required this.title, required this.channel, required this.thumbnail});
}

/// Talks to the same relay the PWA/iOS use. Per-user login returns a session
/// token (relay change a274bee) stored locally and sent back as
/// `Cookie: bkgrnd_session=…`. YouTube-only in v1; streams the proxied endpoint.
class Relay {
  static const base = 'https://bkgrnd.bedl.am';
  static final AudioPlayer player = AudioPlayer();
  static String? _token;

  static Future<void> load() async {
    _token = (await SharedPreferences.getInstance()).getString('session_token');
  }

  static bool get isLoggedIn => _token != null;

  static Map<String, String> get _auth =>
      _token != null ? {'Cookie': 'bkgrnd_session=$_token'} : {};

  static Future<bool> login(String user, String pass) async {
    final resp = await http.post(
      Uri.parse('$base/api/v1/pwa/login'),
      headers: {'content-type': 'application/json'},
      body: jsonEncode({'username': user, 'password': pass}),
    );
    if (resp.statusCode != 200) return false;
    _token = jsonDecode(resp.body)['token'] as String?;
    if (_token != null) {
      (await SharedPreferences.getInstance()).setString('session_token', _token!);
    }
    return _token != null;
  }

  static Future<void> logout() async {
    _token = null;
    (await SharedPreferences.getInstance()).remove('session_token');
    await player.stop();
  }

  static Future<List<Track>> library() async {
    final resp = await http.get(Uri.parse('$base/api/v1/playlists.json'), headers: _auth);
    if (resp.statusCode != 200) return [];
    final doc = jsonDecode(resp.body) as Map<String, dynamic>;
    final out = <Track>[];
    final seen = <String>{};
    for (final pl in (doc['playlists'] as List? ?? [])) {
      for (final it in (pl['items'] as List? ?? [])) {
        final url = (it['url'] as String?) ?? '';
        final key = _videoKey(url); // dedup by video id (parity with other clients)
        if (url.isNotEmpty && seen.add(key)) {
          out.add(Track(
            url: url,
            title: (it['title'] as String?) ?? url,
            channel: (it['channel'] as String?) ?? '',
            thumbnail: (it['thumbnail'] as String?) ?? '',
          ));
        }
      }
    }
    return out;
  }

  static String _streamUrl(String src) =>
      '$base/api/v1/stream?proxy=true&url=${Uri.encodeComponent(src)}';

  static String _videoKey(String url) =>
      RegExp(r'(?:v=|youtu\.be/|/shorts/)([A-Za-z0-9_-]{11})').firstMatch(url)?.group(1) ?? url;

  /// Search, or treat a pasted URL as a direct result.
  static Future<List<Track>> search(String query) async {
    final q = query.trim();
    if (q.isEmpty) return [];
    if (RegExp(r'^https?://').hasMatch(q)) {
      return [Track(url: q, title: q, channel: '', thumbnail: '')];
    }
    final resp = await http.get(
      Uri.parse('$base/api/v1/search?q=${Uri.encodeComponent(q)}'), headers: _auth);
    if (resp.statusCode != 200) return [];
    return (jsonDecode(resp.body) as List)
        .map((o) => Track(
              url: (o['url'] as String?) ?? '',
              title: (o['title'] as String?) ?? (o['url'] as String?) ?? '',
              channel: (o['channel'] as String?) ?? '',
              thumbnail: (o['thumbnail'] as String?) ?? '',
            ))
        .toList();
  }

  /// Resolve a pasted URL's real title/artwork via the relay oEmbed proxy.
  static Future<Track> _enrich(Track t) async {
    if (t.title != t.url) return t;
    try {
      final resp = await http.get(
        Uri.parse('$base/api/v1/meta?url=${Uri.encodeComponent(t.url)}'), headers: _auth);
      if (resp.statusCode == 200) {
        final m = jsonDecode(resp.body) as Map<String, dynamic>;
        final title = (m['title'] as String?) ?? '';
        return Track(
          url: t.url,
          title: title.isNotEmpty ? title : t.title,
          channel: (m['channel'] as String?) ?? t.channel,
          thumbnail: t.thumbnail.isNotEmpty ? t.thumbnail : ((m['thumbnail'] as String?) ?? ''),
        );
      }
    } catch (_) {}
    return t;
  }

  static Future<void> playTrack(Track t) async {
    final track = await _enrich(t); // real title/artwork for pasted URLs
    await player.setAudioSource(AudioSource.uri(
      Uri.parse(_streamUrl(track.url)),
      headers: _auth, // cookie authenticates the proxied stream fetch
      tag: MediaItem(
        id: track.url,
        title: track.title,
        artist: track.channel,
        artUri: track.thumbnail.isNotEmpty ? Uri.tryParse(track.thumbnail) : null,
      ),
    ));
    Relay.player.play();
  }
}

class BkgrndApp extends StatelessWidget {
  const BkgrndApp({super.key});
  @override
  Widget build(BuildContext context) => MaterialApp(
        title: 'bkgrnd',
        debugShowCheckedModeBanner: false,
        theme: ThemeData.dark(useMaterial3: true),
        home: const Gate(),
      );
}

/// Loads the stored token, then shows login or the library.
class Gate extends StatefulWidget {
  const Gate({super.key});
  @override
  State<Gate> createState() => _GateState();
}

class _GateState extends State<Gate> {
  bool _ready = false;

  @override
  void initState() {
    super.initState();
    Relay.load().then((_) => setState(() => _ready = true));
  }

  @override
  Widget build(BuildContext context) {
    if (!_ready) return const Scaffold(body: Center(child: CircularProgressIndicator()));
    return Relay.isLoggedIn
        ? LibraryPage(onLogout: () => setState(() {}))
        : LoginPage(onLoggedIn: () => setState(() {}));
  }
}

class LoginPage extends StatefulWidget {
  final VoidCallback onLoggedIn;
  const LoginPage({super.key, required this.onLoggedIn});
  @override
  State<LoginPage> createState() => _LoginPageState();
}

class _LoginPageState extends State<LoginPage> {
  final _user = TextEditingController();
  final _pass = TextEditingController();
  bool _busy = false;
  String? _error;

  Future<void> _submit() async {
    setState(() { _busy = true; _error = null; });
    final ok = await Relay.login(_user.text.trim(), _pass.text);
    if (!mounted) return;
    if (ok) {
      widget.onLoggedIn();
    } else {
      setState(() { _busy = false; _error = 'Invalid username or password'; });
    }
  }

  @override
  Widget build(BuildContext context) => Scaffold(
        body: Center(
          child: ConstrainedBox(
            constraints: const BoxConstraints(maxWidth: 340),
            child: Padding(
              padding: const EdgeInsets.all(24),
              child: Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  Text('bkgrnd', style: Theme.of(context).textTheme.headlineMedium),
                  const SizedBox(height: 16),
                  TextField(controller: _user, decoration: const InputDecoration(labelText: 'Username')),
                  const SizedBox(height: 8),
                  TextField(controller: _pass, obscureText: true, decoration: const InputDecoration(labelText: 'Password')),
                  if (_error != null) ...[
                    const SizedBox(height: 8),
                    Text(_error!, style: TextStyle(color: Theme.of(context).colorScheme.error)),
                  ],
                  const SizedBox(height: 16),
                  FilledButton(onPressed: _busy ? null : _submit, child: const Text('Sign in')),
                ],
              ),
            ),
          ),
        ),
      );
}

class LibraryPage extends StatefulWidget {
  final VoidCallback onLogout;
  const LibraryPage({super.key, required this.onLogout});
  @override
  State<LibraryPage> createState() => _LibraryPageState();
}

class _LibraryPageState extends State<LibraryPage> {
  late Future<List<Track>> _future;

  @override
  void initState() {
    super.initState();
    _future = Relay.library();
  }

  @override
  Widget build(BuildContext context) => Scaffold(
        appBar: AppBar(
          title: const Text('bkgrnd — Streams'),
          actions: [
            IconButton(
              icon: const Icon(Icons.search),
              onPressed: () => Navigator.of(context).push(
                  MaterialPageRoute(builder: (_) => const SearchPage())),
            ),
            IconButton(
              icon: const Icon(Icons.logout),
              onPressed: () async { await Relay.logout(); widget.onLogout(); },
            ),
          ],
        ),
        bottomNavigationBar: const _MiniPlayer(),
        body: FutureBuilder<List<Track>>(
          future: _future,
          builder: (context, snap) {
            if (!snap.hasData) return const Center(child: CircularProgressIndicator());
            final tracks = snap.data!;
            return ListView.builder(
              itemCount: tracks.length,
              itemBuilder: (context, i) {
                final t = tracks[i];
                return ListTile(
                  leading: t.thumbnail.isEmpty
                      ? const Icon(Icons.music_note)
                      : CachedNetworkImage(imageUrl: t.thumbnail, width: 56, height: 56, fit: BoxFit.cover),
                  title: Text(t.title, maxLines: 2, overflow: TextOverflow.ellipsis),
                  subtitle: t.channel.isEmpty ? null : Text(t.channel),
                  onTap: () => Relay.playTrack(t),
                );
              },
            );
          },
        ),
      );
}

class SearchPage extends StatefulWidget {
  const SearchPage({super.key});
  @override
  State<SearchPage> createState() => _SearchPageState();
}

class _SearchPageState extends State<SearchPage> {
  final _q = TextEditingController();
  List<Track> _results = [];
  bool _busy = false;

  Future<void> _run() async {
    setState(() => _busy = true);
    final r = await Relay.search(_q.text);
    if (!mounted) return;
    setState(() { _results = r; _busy = false; });
  }

  @override
  Widget build(BuildContext context) => Scaffold(
        appBar: AppBar(
          title: TextField(
            controller: _q,
            autofocus: true,
            textInputAction: TextInputAction.search,
            onSubmitted: (_) => _run(),
            decoration: const InputDecoration(
              hintText: 'Search, or paste a YouTube URL',
              border: InputBorder.none,
            ),
          ),
          actions: [IconButton(icon: const Icon(Icons.search), onPressed: _run)],
        ),
        body: _busy
            ? const Center(child: CircularProgressIndicator())
            : ListView.builder(
                itemCount: _results.length,
                itemBuilder: (context, i) {
                  final t = _results[i];
                  return ListTile(
                    leading: t.thumbnail.isEmpty
                        ? const Icon(Icons.music_note)
                        : CachedNetworkImage(imageUrl: t.thumbnail, width: 56, height: 56, fit: BoxFit.cover),
                    title: Text(t.title, maxLines: 2, overflow: TextOverflow.ellipsis),
                    subtitle: t.channel.isEmpty ? null : Text(t.channel),
                    onTap: () {
                      Relay.playTrack(t);
                      Navigator.of(context).pop();
                    },
                  );
                },
              ),
      );
}

class _MiniPlayer extends StatelessWidget {
  const _MiniPlayer();
  @override
  Widget build(BuildContext context) => StreamBuilder<SequenceState?>(
        stream: Relay.player.sequenceStateStream,
        builder: (context, snap) {
          final tag = snap.data?.currentSource?.tag as MediaItem?;
          if (tag == null) return const SizedBox.shrink();
          return Material(
            color: Theme.of(context).colorScheme.surfaceContainerHighest,
            child: ListTile(
              title: Text(tag.title, maxLines: 1, overflow: TextOverflow.ellipsis),
              subtitle: tag.artist == null ? null : Text(tag.artist!, maxLines: 1),
              trailing: StreamBuilder<PlayerState>(
                stream: Relay.player.playerStateStream,
                builder: (context, s) {
                  final playing = s.data?.playing ?? false;
                  return IconButton(
                    icon: Icon(playing ? Icons.pause : Icons.play_arrow),
                    onPressed: () => playing ? Relay.player.pause() : Relay.player.play(),
                  );
                },
              ),
            ),
          );
        },
      );
}
