package org.beo.diagnostics

import android.content.Context
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.util.AttributeSet
import android.view.Choreographer
import android.view.View

/**
 * Animates a bouncing square and reports live FPS via [onFps]. Visible
 * frame-rate instability here is the same underlying signal as "GPU
 * rendering is having trouble" — a controlled, repeatable stand-in for
 * "does the emulator's graphics feel smooth."
 */
class GpuTestView(context: Context, attrs: AttributeSet?) : View(context, attrs) {
    private val paint = Paint().apply { color = Color.rgb(180, 60, 20) }
    private var x = 0f
    private var y = 0f
    private var dx = 6f
    private var dy = 4f
    private val boxSize = 60f

    private var running = false
    private var frameCount = 0
    private var lastFpsReportNanos = 0L
    var onFps: ((Int) -> Unit)? = null

    private val frameCallback = object : Choreographer.FrameCallback {
        override fun doFrame(frameTimeNanos: Long) {
            if (!running) return

            x += dx
            y += dy
            if (x <= 0f || x + boxSize >= width) dx = -dx
            if (y <= 0f || y + boxSize >= height) dy = -dy
            invalidate()

            frameCount++
            if (lastFpsReportNanos == 0L) lastFpsReportNanos = frameTimeNanos
            val elapsedNanos = frameTimeNanos - lastFpsReportNanos
            if (elapsedNanos >= 1_000_000_000L) {
                val fps = (frameCount * 1_000_000_000.0 / elapsedNanos).toInt()
                onFps?.invoke(fps)
                frameCount = 0
                lastFpsReportNanos = frameTimeNanos
            }

            Choreographer.getInstance().postFrameCallback(this)
        }
    }

    fun start() {
        if (running) return
        running = true
        frameCount = 0
        lastFpsReportNanos = 0L
        Choreographer.getInstance().postFrameCallback(frameCallback)
    }

    fun stop() {
        running = false
        Choreographer.getInstance().removeFrameCallback(frameCallback)
    }

    override fun onDraw(canvas: Canvas) {
        super.onDraw(canvas)
        canvas.drawRect(x, y, x + boxSize, y + boxSize, paint)
    }
}
