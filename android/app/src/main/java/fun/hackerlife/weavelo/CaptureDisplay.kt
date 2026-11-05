package `fun`.hackerlife.weavelo

import android.hardware.display.DisplayManager
import android.media.projection.MediaProjection
import android.os.Handler
import android.os.Looper
import android.view.Surface
import `fun`.hackerlife.weavelo.managed.IManagedScreen

/** Both native capture backends feed the same GPU Surface and encoder. */
internal interface CaptureDisplay {
    var surface: Surface?
    fun resize(width: Int, height: Int, density: Int)
    fun release()

    companion object {
        fun projection(capture: MediaProjection, width: Int, height: Int, density: Int): CaptureDisplay {
            val display = capture.createVirtualDisplay("Weavelo", width, height, density,
                DisplayManager.VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR, null, null, Handler(Looper.getMainLooper()))
            return object : CaptureDisplay {
                override var surface: Surface?
                    get() = display.surface
                    set(value) { display.surface = value }
                override fun resize(width: Int, height: Int, density: Int) = display.resize(width, height, density)
                override fun release() = display.release()
            }
        }
        fun managed(host: IManagedScreen, width: Int, height: Int, density: Int): CaptureDisplay {
            val id = host.createMirror(width, height, density)
            return object : CaptureDisplay {
                override var surface: Surface? = null
                    set(value) { host.setSurface(id, value); field = value }
                override fun resize(width: Int, height: Int, density: Int) = host.resize(id, width, height, density)
                override fun release() = host.release(id)
            }
        }
    }
}
