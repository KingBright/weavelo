package `fun`.hackerlife.weavelo

import android.graphics.SurfaceTexture
import android.opengl.EGL14
import android.opengl.EGLExt
import android.opengl.GLES11Ext
import android.opengl.GLES20
import android.os.Handler
import android.os.HandlerThread
import android.view.Surface
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.TreeMap
import java.util.concurrent.atomic.AtomicBoolean

/**
 * One native OES texture and EGL context. The decoder stays on Surface input;
 * the GPU draws each ROI into its full-source rectangle before local UI zoom.
 * The same primitive can sample a source ROI into an encoder's native Surface.
 * There is no CPU image readback and no extra full/detail video subscription.
 */
internal class GpuVideoSurface(
    private val output: Surface?,
    sourceWidth: Int,
    sourceHeight: Int,
    private val captureClockUs: (() -> Long)? = null,
    private val sourceRotation: Int = 0,
    private val onFrame: (VideoFrameGeometry?) -> Unit = {}
) : AutoCloseable {
    data class Presentation(val ready: Boolean = false, val geometry: VideoFrameGeometry? = null)
    @Volatile var presentation = Presentation()
        private set
    var outputFrames = 0L
        private set
    private val thread = Thread.currentThread()
    private val display = DisplayOwner.acquire()
    private val callbacks = HandlerThread("weavelo-texture-events").apply { start() }
    private val available = AtomicBoolean(false)
    private val signal = Object()
    private val metadata = TreeMap<Long, VideoFrameGeometry?>()
    private var lastRenderedPts = Long.MIN_VALUE
    private var lastRegisteredPts = Long.MIN_VALUE
    private var ignoreThroughPts = Long.MIN_VALUE
    private var context = EGL14.EGL_NO_CONTEXT
    private var window = EGL14.EGL_NO_SURFACE
    private var config: android.opengl.EGLConfig? = null
    internal data class Output(val surface: android.opengl.EGLSurface, val width: Int, val height: Int)
    private val outputs = mutableSetOf<Output>()
    var sourceReady = false
        private set
    var sourceCaptureUs = 0L
        private set
    var sourceDataSpace = 0
        private set
    private var sourceTimestampNs: Long? = null
    private var program = 0
    private var texture = 0
    private var image: SurfaceTexture? = null
    private var producer: Surface? = null
    val input: Surface get() = checkNotNull(producer)
    private var closed = false
    private val matrix = FloatArray(16)
    private val sourceMatrix = FloatArray(16)
    private val sourceTransform = when (sourceRotation) {
        1 -> floatArrayOf(0f,-1f,0f,0f, 1f,0f,0f,0f, 0f,0f,1f,0f, 0f,1f,0f,1f)
        2 -> floatArrayOf(-1f,0f,0f,0f, 0f,-1f,0f,0f, 0f,0f,1f,0f, 1f,1f,0f,1f)
        3 -> floatArrayOf(0f,1f,0f,0f, -1f,0f,0f,0f, 0f,0f,1f,0f, 1f,0f,0f,1f)
        else -> null
    }
    private val quad = ByteBuffer.allocateDirect(16 * 4).order(ByteOrder.nativeOrder()).asFloatBuffer().apply {
        // Position and texture coordinates, both in normal OpenGL orientation.
        put(floatArrayOf(-1f,-1f,0f,0f, 1f,-1f,1f,0f, -1f,1f,0f,1f, 1f,1f,1f,1f)); position(0)
    }
    init {
        try {
            require(output?.isValid != false && sourceWidth > 1 && sourceHeight > 1 && sourceRotation in 0..3)
            val configs = arrayOfNulls<android.opengl.EGLConfig>(1)
            val count = IntArray(1)
            val attrs = intArrayOf(EGL14.EGL_RED_SIZE,8,EGL14.EGL_GREEN_SIZE,8,EGL14.EGL_BLUE_SIZE,8,EGL14.EGL_ALPHA_SIZE,8,
                EGL14.EGL_RENDERABLE_TYPE,EGL14.EGL_OPENGL_ES2_BIT,EGL14.EGL_SURFACE_TYPE,EGL14.EGL_WINDOW_BIT or EGL14.EGL_PBUFFER_BIT,
                0x3142,1,EGL14.EGL_NONE) // EGL_RECORDABLE_ANDROID
            check(EGL14.eglChooseConfig(display,attrs,0,configs,0,1,count,0) && count[0] > 0) { "No native video EGL configuration" }
            config = checkNotNull(configs[0])
            context = EGL14.eglCreateContext(display,configs[0],EGL14.EGL_NO_CONTEXT,intArrayOf(EGL14.EGL_CONTEXT_CLIENT_VERSION,2,EGL14.EGL_NONE),0)
            check(context != EGL14.EGL_NO_CONTEXT)
            window = if (output != null) EGL14.eglCreateWindowSurface(display,config,output,intArrayOf(EGL14.EGL_NONE),0)
                else EGL14.eglCreatePbufferSurface(display,config,intArrayOf(EGL14.EGL_WIDTH,1,EGL14.EGL_HEIGHT,1,EGL14.EGL_NONE),0)
            check(window != EGL14.EGL_NO_SURFACE && EGL14.eglMakeCurrent(display,window,window,context))
            program = GLES20.glCreateProgram()
            for ((type, source) in listOf(GLES20.GL_VERTEX_SHADER to VERTEX, GLES20.GL_FRAGMENT_SHADER to FRAGMENT)) {
                val shader = GLES20.glCreateShader(type)
                GLES20.glShaderSource(shader, source); GLES20.glCompileShader(shader)
                val status = IntArray(1); GLES20.glGetShaderiv(shader,GLES20.GL_COMPILE_STATUS,status,0)
                check(status[0] != 0) { "Video shader failed: ${GLES20.glGetShaderInfoLog(shader)}" }
                GLES20.glAttachShader(program,shader); GLES20.glDeleteShader(shader)
            }
            GLES20.glLinkProgram(program)
            val linked = IntArray(1); GLES20.glGetProgramiv(program,GLES20.GL_LINK_STATUS,linked,0)
            check(linked[0] != 0) { "Video program failed: ${GLES20.glGetProgramInfoLog(program)}" }
            val names = IntArray(1); GLES20.glGenTextures(1,names,0); texture = names[0]
            GLES20.glBindTexture(GLES11Ext.GL_TEXTURE_EXTERNAL_OES,texture)
            GLES20.glTexParameteri(GLES11Ext.GL_TEXTURE_EXTERNAL_OES,GLES20.GL_TEXTURE_MIN_FILTER,GLES20.GL_LINEAR)
            GLES20.glTexParameteri(GLES11Ext.GL_TEXTURE_EXTERNAL_OES,GLES20.GL_TEXTURE_MAG_FILTER,GLES20.GL_LINEAR)
            GLES20.glTexParameteri(GLES11Ext.GL_TEXTURE_EXTERNAL_OES,GLES20.GL_TEXTURE_WRAP_S,GLES20.GL_CLAMP_TO_EDGE)
            GLES20.glTexParameteri(GLES11Ext.GL_TEXTURE_EXTERNAL_OES,GLES20.GL_TEXTURE_WRAP_T,GLES20.GL_CLAMP_TO_EDGE)
            image = SurfaceTexture(texture).also { st ->
                st.setDefaultBufferSize(if (sourceRotation % 2 == 0) sourceWidth else sourceHeight,
                    if (sourceRotation % 2 == 0) sourceHeight else sourceWidth)
                st.setOnFrameAvailableListener({ synchronized(signal) { available.set(true); signal.notifyAll() } },Handler(callbacks.looper))
                producer = Surface(st)
            }
        } catch (error: Throwable) { close(); throw error }
    }
    private fun checkThread() = check(Thread.currentThread() === thread && !closed) { "Native video context owner changed" }
    fun addOutput(surface: Surface): Output {
        checkThread(); check(output == null && surface.isValid)
        val target=EGL14.eglCreateWindowSurface(display,config,surface,intArrayOf(EGL14.EGL_NONE),0)
        check(target != EGL14.EGL_NO_SURFACE) { "Cannot import hardware encoder Surface" }
        try {
            val w=IntArray(1);val h=IntArray(1)
            check(EGL14.eglQuerySurface(display,target,EGL14.EGL_WIDTH,w,0) && EGL14.eglQuerySurface(display,target,EGL14.EGL_HEIGHT,h,0))
            return Output(target,w[0],h[0]).also { outputs.add(it) }
        } catch (error: Throwable) { EGL14.eglDestroySurface(display,target); throw error }
    }
    fun removeOutput(target: Output) {
        checkThread()
        if (outputs.remove(target)) {
            check(EGL14.eglMakeCurrent(display,window,window,context))
            EGL14.eglDestroySurface(display,target.surface)
        }
    }
    /** Latest raw image only. A cached redraw retains this ingress clock. */
    fun latchSource(): Boolean {
        checkThread(); check(output == null)
        if (!available.getAndSet(false)) return false
        check(EGL14.eglMakeCurrent(display,window,window,context))
        val st=checkNotNull(image);st.updateTexImage();st.getTransformMatrix(matrix)
        sourceTransform?.let { android.opengl.Matrix.multiplyMM(sourceMatrix,0,matrix,0,it,0) }
        val timestamp=st.timestamp
        if (sourceTimestampNs == timestamp) return false
        check(timestamp > 0) { "Screen buffer has no producer timestamp" }
        sourceTimestampNs=timestamp
        // SurfaceTexture timestamps have producer-specific timebases. Record
        // the actual GPU ingress observation, not a guessed physical capture clock.
        sourceCaptureUs=checkNotNull(captureClockUs) { "Screen GPU ingress clock is unavailable" }()
        sourceDataSpace=if (android.os.Build.VERSION.SDK_INT >= 33) st.dataSpace else 0
        sourceReady=true;return true
    }
    fun drawSource(target: Output, geometry: VideoFrameGeometry, ptsUs: Long) {
        checkThread();check(output == null && sourceReady && target in outputs && geometry.valid())
        check(target.width == geometry.width && target.height == geometry.height)
        val sample=floatArrayOf(geometry.x.toFloat()/geometry.sourceWidth,
            1f-(geometry.y+geometry.cropHeight).toFloat()/geometry.sourceHeight,
            geometry.cropWidth.toFloat()/geometry.sourceWidth,geometry.cropHeight.toFloat()/geometry.sourceHeight)
        draw(if (sourceTransform == null) matrix else sourceMatrix,floatArrayOf(0f,0f,1f,1f),sample,Math.multiplyExact(ptsUs,1000),target)
    }
    fun awaitSource(timeoutMs: Long) {
        checkThread();check(output == null)
        if (timeoutMs > 0) synchronized(signal) { if (!available.get()) signal.wait(timeoutMs) }
    }
    fun register(ptsUs: Long, geometry: VideoFrameGeometry?) {
        checkThread()
        while (metadata.size >= 96) {
            metadata.pollFirstEntry()
        }
        check(!metadata.containsKey(ptsUs)) { "Video frame timestamp was reused" }
        metadata[ptsUs] = geometry
        lastRegisteredPts = ptsUs
    }
    fun reset(resetCount: Boolean = false) {
        checkThread(); ignoreThroughPts = lastRegisteredPts; metadata.clear(); presentation = Presentation()
        if (resetCount) outputFrames = 0
    }
    /** Called by the existing decoder worker; callbacks only signal availability. */
    fun renderPending() {
        checkThread()
        if (!available.getAndSet(false)) return
        check(EGL14.eglMakeCurrent(display,window,window,context))
        val st = checkNotNull(image)
        st.updateTexImage(); st.getTransformMatrix(matrix)
        val pts = st.timestamp / 1000
        if (pts <= ignoreThroughPts || pts <= lastRenderedPts) return // only already-decoded obsolete output
        check(metadata.containsKey(pts)) { "Decoded image has no matching viewport geometry" }
        val g = metadata[pts]
        metadata.headMap(pts,true).clear()
        val draw = g?.let { floatArrayOf(it.x.toFloat()/it.sourceWidth,
            1f-(it.y+it.cropHeight).toFloat()/it.sourceHeight,it.cropWidth.toFloat()/it.sourceWidth,it.cropHeight.toFloat()/it.sourceHeight) }
            ?: floatArrayOf(0f,0f,1f,1f)
        draw(matrix,draw,floatArrayOf(0f,0f,1f,1f),null)
        presentation = Presentation(true,g); outputFrames++; onFrame(g)
        lastRenderedPts = pts
    }
    private fun draw(textureMatrix: FloatArray, destination: FloatArray, sample: FloatArray, timestampNs: Long?, target: Output? = null) {
        val targetSurface=target?.surface ?: window
        check(EGL14.eglMakeCurrent(display,targetSurface,targetSurface,context))
        val w=IntArray(1);val h=IntArray(1)
        if (target == null) check(EGL14.eglQuerySurface(display,window,EGL14.EGL_WIDTH,w,0) && EGL14.eglQuerySurface(display,window,EGL14.EGL_HEIGHT,h,0))
        else { w[0]=target.width;h[0]=target.height }
        GLES20.glViewport(0,0,w[0],h[0]);GLES20.glClearColor(0f,0f,0f,1f);GLES20.glClear(GLES20.GL_COLOR_BUFFER_BIT)
        GLES20.glUseProgram(program)
        val position=GLES20.glGetAttribLocation(program,"position");val tex=GLES20.glGetAttribLocation(program,"texCoord")
        quad.position(0);GLES20.glVertexAttribPointer(position,2,GLES20.GL_FLOAT,false,16,quad);GLES20.glEnableVertexAttribArray(position)
        quad.position(2);GLES20.glVertexAttribPointer(tex,2,GLES20.GL_FLOAT,false,16,quad);GLES20.glEnableVertexAttribArray(tex)
        GLES20.glUniformMatrix4fv(GLES20.glGetUniformLocation(program,"textureMatrix"),1,false,textureMatrix,0)
        GLES20.glUniform4fv(GLES20.glGetUniformLocation(program,"destination"),1,destination,0)
        GLES20.glUniform4fv(GLES20.glGetUniformLocation(program,"sampleRect"),1,sample,0)
        GLES20.glActiveTexture(GLES20.GL_TEXTURE0);GLES20.glBindTexture(GLES11Ext.GL_TEXTURE_EXTERNAL_OES,texture)
        GLES20.glUniform1i(GLES20.glGetUniformLocation(program,"image"),0)
        GLES20.glDrawArrays(GLES20.GL_TRIANGLE_STRIP,0,4)
        check(GLES20.glGetError()==GLES20.GL_NO_ERROR) { "Native viewport drawing failed" }
        if (timestampNs != null) check(EGLExt.eglPresentationTimeANDROID(display,targetSurface,timestampNs))
        check(EGL14.eglSwapBuffers(display,targetSurface)) { "Native video surface was lost" }
    }
    override fun close() {
        if (closed) return
        check(Thread.currentThread() === thread)
        if (context != EGL14.EGL_NO_CONTEXT && window != EGL14.EGL_NO_SURFACE)
            EGL14.eglMakeCurrent(display,window,window,context)
        presentation = Presentation(); closed = true
        image?.setOnFrameAvailableListener(null); producer?.release(); producer=null; image?.release(); image=null
        if (program != 0) GLES20.glDeleteProgram(program)
        if (texture != 0) GLES20.glDeleteTextures(1,intArrayOf(texture),0)
        if (display != EGL14.EGL_NO_DISPLAY) {
            for (target in outputs) EGL14.eglDestroySurface(display,target.surface)
            outputs.clear()
            EGL14.eglMakeCurrent(display,EGL14.EGL_NO_SURFACE,EGL14.EGL_NO_SURFACE,EGL14.EGL_NO_CONTEXT)
            if (window != EGL14.EGL_NO_SURFACE) EGL14.eglDestroySurface(display,window)
            if (context != EGL14.EGL_NO_CONTEXT) EGL14.eglDestroyContext(display,context)
            EGL14.eglReleaseThread(); DisplayOwner.release()
        }
        callbacks.quitSafely()
        try { callbacks.join(1000) }
        catch (_: InterruptedException) { Thread.currentThread().interrupt() }
    }
    companion object {
        private const val VERTEX = """
            attribute vec2 position; attribute vec2 texCoord;
            uniform mat4 textureMatrix; uniform vec4 destination; uniform vec4 sampleRect;
            varying vec2 uv;
            void main() {
                gl_Position=vec4(2.0*(destination.xy+(position+1.0)*0.5*destination.zw)-1.0,0.0,1.0);
                uv=(textureMatrix*vec4(sampleRect.xy+texCoord*sampleRect.zw,0.0,1.0)).xy;
            }
        """
        private const val FRAGMENT = """
            #extension GL_OES_EGL_image_external : require
            precision highp float;
            uniform samplerExternalOES image; varying vec2 uv;
            void main() { gl_FragColor=texture2D(image,uv); }
        """
    }
}

/** Independent stream contexts share one display; ending one cannot terminate others. */
private object DisplayOwner {
    private var owners = 0
    private var display = EGL14.EGL_NO_DISPLAY
    @Synchronized fun acquire(): android.opengl.EGLDisplay {
        if (owners == 0) {
            val candidate = EGL14.eglGetDisplay(EGL14.EGL_DEFAULT_DISPLAY)
            check(candidate != EGL14.EGL_NO_DISPLAY && EGL14.eglInitialize(candidate,null,0,null,0)) { "Cannot initialize native video EGL" }
            display = candidate
        }
        owners++; return display
    }
    @Synchronized fun release() {
        check(owners > 0)
        if (--owners == 0) { EGL14.eglTerminate(display); display = EGL14.EGL_NO_DISPLAY }
    }
}
