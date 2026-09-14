package org.beo.diagnostics

import android.media.AudioAttributes
import android.media.AudioFocusRequest
import android.media.AudioFormat
import android.media.AudioManager
import android.media.AudioTrack
import android.media.MediaPlayer
import android.os.Bundle
import android.os.StatFs
import android.widget.Button
import android.widget.TextView
import androidx.appcompat.app.AppCompatActivity
import androidx.core.view.ViewCompat
import androidx.core.view.WindowInsetsCompat
import java.io.File
import java.net.HttpURLConnection
import java.net.URL
import kotlin.math.sin

/**
 * One Activity, four independent checks — audio, GPU rendering, network,
 * storage — each with its own Run/Stop button and status line. Built to
 * give a controlled, repeatable signal for each of these (a synthesized
 * tone instead of "play something in the browser and listen", a live FPS
 * counter instead of "does scrolling feel smooth") so comparing behavior
 * across Beo's own `-gpu`/`-feature` emulator flag experiments doesn't
 * rely on subjective impressions.
 */
class MainActivity : AppCompatActivity() {

    private var audioThread: Thread? = null
    @Volatile private var audioPlaying = false

    private var musicPlayer: MediaPlayer? = null
    private var musicFocusRequest: AudioFocusRequest? = null

    private lateinit var gpuView: GpuTestView
    private var gpuRunning = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_main)

        // Targeting SDK 35+ makes the system draw the status/navigation
        // bars *over* the app's content by default (edge-to-edge is
        // enforced, not opt-in) — confirmed live on a real tablet: the
        // bottom of the Storage section rendered directly behind the
        // navigation bar, invisible and unreachable, which looked exactly
        // like "the app doesn't scroll" (there was nothing left to scroll
        // to — the content was already fully laid out, just hidden behind
        // opaque system chrome). Padding the scroll root by the real system
        // bar insets, rather than opting out via the now-deprecated
        // setDecorFitsSystemWindows(true), since Google's own direction is
        // that the opt-out won't keep being honored in future versions.
        val scrollRoot = findViewById<android.widget.ScrollView>(R.id.scrollRoot)
        val initialPaddingLeft = scrollRoot.paddingLeft
        val initialPaddingTop = scrollRoot.paddingTop
        val initialPaddingRight = scrollRoot.paddingRight
        val initialPaddingBottom = scrollRoot.paddingBottom
        ViewCompat.setOnApplyWindowInsetsListener(scrollRoot) { view, insets ->
            val bars = insets.getInsets(WindowInsetsCompat.Type.systemBars())
            view.setPadding(
                initialPaddingLeft + bars.left,
                initialPaddingTop + bars.top,
                initialPaddingRight + bars.right,
                initialPaddingBottom + bars.bottom
            )
            insets
        }

        val btnAudio = findViewById<Button>(R.id.btnAudio)
        val txtAudio = findViewById<TextView>(R.id.txtAudio)
        btnAudio.setOnClickListener {
            if (audioPlaying) {
                stopAudio()
                btnAudio.text = "Play Tone"
                txtAudio.text = "Stopped."
            } else {
                startAudio(txtAudio)
                btnAudio.text = "Stop Tone"
            }
        }

        val btnMusic = findViewById<Button>(R.id.btnMusic)
        val txtMusic = findViewById<TextView>(R.id.txtMusic)
        btnMusic.setOnClickListener {
            if (musicPlayer != null) {
                stopMusic()
                btnMusic.text = "Play Music File"
                txtMusic.text = "Stopped."
            } else {
                startMusic(txtMusic) { btnMusic.text = "Play Music File" }
                btnMusic.text = "Stop Music File"
            }
        }

        gpuView = findViewById(R.id.gpuView)
        val btnGpu = findViewById<Button>(R.id.btnGpu)
        val txtGpu = findViewById<TextView>(R.id.txtGpu)
        gpuView.onFps = { fps -> runOnUiThread { txtGpu.text = "Running — $fps fps" } }
        btnGpu.setOnClickListener {
            if (gpuRunning) {
                gpuView.stop()
                gpuRunning = false
                btnGpu.text = "Start Animation"
                txtGpu.text = "Stopped."
            } else {
                gpuView.start()
                gpuRunning = true
                btnGpu.text = "Stop Animation"
                txtGpu.text = "Running…"
            }
        }

        val btnNetwork = findViewById<Button>(R.id.btnNetwork)
        val txtNetwork = findViewById<TextView>(R.id.txtNetwork)
        btnNetwork.setOnClickListener {
            txtNetwork.text = "Checking…"
            checkNetwork(txtNetwork)
        }

        val btnStorage = findViewById<Button>(R.id.btnStorage)
        val txtStorage = findViewById<TextView>(R.id.txtStorage)
        btnStorage.setOnClickListener {
            txtStorage.text = checkStorage()
        }
    }

    override fun onDestroy() {
        super.onDestroy()
        stopAudio()
        stopMusic()
        gpuView.stop()
    }

    // --- Music file (native playback, no browser/WebView involved) ---

    private fun startMusic(status: TextView, onFinished: () -> Unit) {
        val attrs = AudioAttributes.Builder()
            .setUsage(AudioAttributes.USAGE_MEDIA)
            .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
            .build()

        // MediaPlayer.create() alone never requests AudioFocus and, without
        // an explicit AudioAttributes, reports usage=USAGE_UNKNOWN in
        // dumpsys — confirmed live: the audio-file e2e test's focus check
        // (looking for a real, active player) initially found no signal at
        // all despite genuinely-playing audio, because this app was never
        // formally requesting focus like a well-behaved media app should.
        val audioManager = getSystemService(AUDIO_SERVICE) as AudioManager
        val focusRequest = AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN)
            .setAudioAttributes(attrs)
            .build()
        musicFocusRequest = focusRequest
        audioManager.requestAudioFocus(focusRequest)

        // MediaPlayer.create() is documented to return null on failure
        // (a corrupt/missing resource, no free decoder, etc.) — treating
        // it as always non-null would crash on the very next line instead
        // of reporting a real error through the status line like every
        // other check in this app does.
        val player = MediaPlayer.create(this, R.raw.sample_song, attrs, 0)
        if (player == null) {
            musicFocusRequest?.let { audioManager.abandonAudioFocusRequest(it) }
            musicFocusRequest = null
            status.text = "Failed to create MediaPlayer for sample_song.wav"
            onFinished()
            return
        }
        musicPlayer = player
        // Loops rather than stopping on completion — the sample clip is
        // only ~6s, and sustained playback (not a single short clip) is
        // what an underrun/glitch check over a real capture window needs.
        player.isLooping = true
        player.setOnErrorListener { _, what, extra ->
            stopMusic()
            status.text = "Playback error ($what, $extra)"
            onFinished()
            true
        }
        player.start()
        status.text = "Playing sample_song.wav (looping)…"
    }

    private fun stopMusic() {
        musicPlayer?.let {
            try {
                if (it.isPlaying) it.stop()
            } catch (_: IllegalStateException) {
                // already stopped/released — nothing to do
            }
            it.release()
        }
        musicPlayer = null
        musicFocusRequest?.let {
            (getSystemService(AUDIO_SERVICE) as AudioManager).abandonAudioFocusRequest(it)
        }
        musicFocusRequest = null
    }

    // --- Audio ---

    private fun startAudio(status: TextView) {
        audioPlaying = true
        audioThread = Thread {
            val sampleRate = 44100
            val freqHz = 440.0
            val bufferSize = AudioTrack.getMinBufferSize(
                sampleRate, AudioFormat.CHANNEL_OUT_MONO, AudioFormat.ENCODING_PCM_16BIT
            )
            val track = AudioTrack.Builder()
                .setAudioAttributes(
                    AudioAttributes.Builder()
                        .setUsage(AudioAttributes.USAGE_MEDIA)
                        .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
                        .build()
                )
                .setAudioFormat(
                    AudioFormat.Builder()
                        .setSampleRate(sampleRate)
                        .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                        .setChannelMask(AudioFormat.CHANNEL_OUT_MONO)
                        .build()
                )
                .setBufferSizeInBytes(bufferSize)
                .setTransferMode(AudioTrack.MODE_STREAM)
                .build()

            val chunk = ShortArray(sampleRate / 10) // 100ms chunks
            var phase = 0.0
            val phaseStep = 2.0 * Math.PI * freqHz / sampleRate
            val startNanos = System.nanoTime()

            track.play()
            while (audioPlaying) {
                for (i in chunk.indices) {
                    chunk[i] = (sin(phase) * Short.MAX_VALUE * 0.6).toInt().toShort()
                    phase += phaseStep
                }
                track.write(chunk, 0, chunk.size)
                val elapsedSec = (System.nanoTime() - startNanos) / 1_000_000_000
                runOnUiThread { status.text = "Playing 440Hz tone — ${elapsedSec}s" }
            }
            track.stop()
            track.release()
        }
        audioThread?.start()
    }

    private fun stopAudio() {
        audioPlaying = false
        audioThread?.join(500)
        audioThread = null
    }

    // --- Network ---

    private fun checkNetwork(status: TextView) {
        Thread {
            val result = try {
                val start = System.nanoTime()
                val conn = URL("https://connectivitycheck.gstatic.com/generate_204")
                    .openConnection() as HttpURLConnection
                conn.requestMethod = "HEAD"
                conn.connectTimeout = 5000
                conn.readTimeout = 5000
                val code = conn.responseCode
                val ms = (System.nanoTime() - start) / 1_000_000
                conn.disconnect()
                if (code == 204) "OK — ${ms}ms" else "Unexpected response: HTTP $code"
            } catch (e: Exception) {
                "Failed: ${e.message}"
            }
            runOnUiThread { status.text = result }
        }.start()
    }

    // --- Storage ---

    private fun checkStorage(): String {
        return try {
            val testFile = File(filesDir, "beo_diagnostics_test.txt")
            val payload = "beo-diagnostics-${System.currentTimeMillis()}"
            testFile.writeText(payload)
            val readBack = testFile.readText()
            testFile.delete()
            val ok = readBack == payload
            val stat = StatFs(filesDir.path)
            val freeMb = stat.availableBytes / (1024 * 1024)
            if (ok) "OK — write/read/delete succeeded, ${freeMb}MB free"
            else "FAILED — read-back didn't match what was written"
        } catch (e: Exception) {
            "Failed: ${e.message}"
        }
    }
}
