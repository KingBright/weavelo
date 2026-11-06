package `fun`.hackerlife.weavelo

import android.app.Activity
import android.app.ActivityManager
import android.content.Context
import android.graphics.*
import android.net.Uri
import org.json.JSONArray
import org.json.JSONObject

enum class DeviceBadgePreset(val wireValue: String, val displayName: String) {
    COMPUTER("computer", "Computer"), MAC_BOOK("mac_book", "MacBook"), MAC_STUDIO("mac_studio", "Mac Studio"),
    WINDOWS("windows", "Windows PC"), LINUX("linux", "Linux PC"), PHONE("phone", "Android phone");

    companion object {
        fun fromWire(value: String?): DeviceBadgePreset =
            values().firstOrNull { it.wireValue == value } ?: COMPUTER
    }
}

data class RemoteAppearance(val application: Bitmap?, val preset: DeviceBadgePreset, val custom: Bitmap?)

/** Immutable assets, decoded once per source response; never on a video frame. */
object DeviceArtwork {
    private var preferences: android.content.SharedPreferences? = null
    fun initialize(context: Context) { preferences = context.getSharedPreferences("device_artwork", Context.MODE_PRIVATE) }
    fun savedJson(): Any = org.json.JSONTokener(preferences?.getString("badge", "\"automatic\"") ?: "\"automatic\"").nextValue()
    fun save(value: Any) {
        val encoded = if (value is String) JSONObject.quote(value) else value.toString()
        check(preferences?.edit()?.putString("badge", encoded)?.commit() == true) { "Could not save device badge" }
        // A running publisher observes the change now; a later start reads the saved value.
        runCatching { PublisherClient.action(41, encoded) }.onFailure {
            if (it.message != "screen sharing is not running") throw it
        }
    }
    private fun bitmap(icon: JSONObject?, edge: Int, budget: Int): Bitmap? {
        if (icon == null) return null
        val png = icon.getJSONArray("png")
        require(png.length() in 1..budget)
        val bytes = ByteArray(png.length()) { png.getInt(it).toByte() }
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
        require(bounds.outWidth in 1..edge && bounds.outHeight in 1..edge)
        return BitmapFactory.decodeByteArray(bytes, 0, bytes.size)
    }
    fun decode(value: JSONObject): RemoteAppearance {
        val device = value.getJSONObject("device")
        return RemoteAppearance(bitmap(value.optJSONObject("application"), 256, 262144),
            DeviceBadgePreset.fromWire(device.optString("preset", "computer")), bitmap(device.optJSONObject("custom"), 64, 16384))
    }
    fun importImage(context: Context, uri: Uri): JSONObject {
        val bytes = context.contentResolver.openInputStream(uri)?.use { stream ->
            val output = java.io.ByteArrayOutputStream()
            val buffer = ByteArray(8192)
            while (true) {
                val count = stream.read(buffer)
                if (count < 0) break
                require(output.size() + count <= 2 * 1024 * 1024) { "Choose an image smaller than 2 MiB" }
                output.write(buffer, 0, count)
            }
            output.toByteArray()
        } ?: error("Could not open image")
        val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
        BitmapFactory.decodeByteArray(bytes, 0, bytes.size, bounds)
        require(bounds.outWidth in 1..1024 && bounds.outHeight in 1..1024) { "Choose an image up to 1024 × 1024" }
        val original = BitmapFactory.decodeByteArray(bytes, 0, bytes.size) ?: error("Unsupported image")
        val image = Bitmap.createBitmap(64, 64, Bitmap.Config.ARGB_8888)
        val scale = 64f / maxOf(original.width, original.height)
        val w = original.width * scale; val h = original.height * scale
        Canvas(image).drawBitmap(original, null, RectF((64-w)/2, (64-h)/2, (64+w)/2, (64+h)/2), Paint(Paint.FILTER_BITMAP_FLAG))
        original.recycle()
        val colors = IntArray(4096); image.getPixels(colors, 0, 64, 0, 0, 64, 64); image.recycle()
        val rgba = JSONArray()
        colors.forEach { color -> rgba.put(Color.red(color)); rgba.put(Color.green(color)); rgba.put(Color.blue(color)); rgba.put(Color.alpha(color)) }
        val icon = JSONObject(PublisherClient.action(42, JSONObject().put("width",64).put("height",64).put("rgba",rgba).toString()))
        return JSONObject().put("custom",icon)
    }
    fun badge(preset: DeviceBadgePreset, custom: Bitmap? = null): Bitmap {
        if (custom != null) return custom
        val bitmap = Bitmap.createBitmap(128,128,Bitmap.Config.ARGB_8888)
        val canvas = Canvas(bitmap); val paint = Paint(Paint.ANTI_ALIAS_FLAG)
        paint.color=Color.rgb(24,52,59); canvas.drawRoundRect(4f,4f,124f,124f,30f,30f,paint)
        paint.color=Color.rgb(139,228,229)
        when(preset) {
            DeviceBadgePreset.PHONE -> {
                canvas.drawRoundRect(39f,22f,89f,107f,9f,9f,paint)
                paint.color=Color.rgb(49,139,153); canvas.drawRoundRect(44f,28f,84f,94f,4f,4f,paint)
                canvas.drawRoundRect(55f,99f,73f,102f,1.5f,1.5f,paint)
            }
            DeviceBadgePreset.MAC_BOOK -> {
                canvas.drawRoundRect(27f,30f,101f,85f,5f,5f,paint)
                canvas.drawRoundRect(17f,89f,111f,98f,4f,4f,paint)
                paint.color=Color.rgb(49,139,153); canvas.drawRect(32f,35f,96f,79f,paint)
            }
            DeviceBadgePreset.MAC_STUDIO -> {
                canvas.drawRoundRect(24f,39f,104f,91f,12f,12f,paint)
                paint.color=Color.rgb(49,139,153); canvas.drawCircle(90f,78f,3f,paint)
            }
            DeviceBadgePreset.WINDOWS -> for(y in listOf(32f,67f)) for(x in listOf(32f,67f)) canvas.drawRect(x,y,x+29,y+29,paint)
            DeviceBadgePreset.LINUX -> {
                paint.color=Color.BLACK; canvas.drawOval(37f,21f,91f,104f,paint)
                paint.color=Color.WHITE; canvas.drawOval(45f,49f,83f,95f,paint)
                canvas.drawCircle(56f,43f,6f,paint); canvas.drawCircle(72f,43f,6f,paint)
                paint.color=Color.rgb(242,181,66); canvas.drawOval(53f,51f,75f,61f,paint)
                canvas.drawOval(31f,95f,60f,106f,paint); canvas.drawOval(68f,95f,97f,106f,paint)
            }
            DeviceBadgePreset.COMPUTER -> {
                canvas.drawRoundRect(24f,29f,104f,86f,6f,6f,paint)
                canvas.drawRect(58f,86f,70f,99f,paint); canvas.drawRect(44f,98f,84f,103f,paint)
            }
        }
        return bitmap
    }
    fun task(activity: Activity, pane: WorkspacePane, device: String) {
        val appearance = pane.appearance
        val app = appearance?.application ?: pane.source.icon
        val badge = badge(appearance?.preset ?: DeviceBadgePreset.COMPUTER, appearance?.custom)
        val icon = Bitmap.createBitmap(256,256,Bitmap.Config.ARGB_8888)
        val canvas = Canvas(icon); val paint = Paint(Paint.ANTI_ALIAS_FLAG or Paint.FILTER_BITMAP_FLAG)
        val main = app ?: badge
        val scale = 256f / maxOf(main.width, main.height)
        val w=main.width*scale; val h=main.height*scale
        canvas.drawBitmap(main,null,RectF((256-w)/2,(256-h)/2,(256+w)/2,(256+h)/2),paint)
        if (app != null) canvas.drawBitmap(badge,null,RectF(158f,145f,250f,237f),paint)
        paint.color=Color.rgb(24,52,59); canvas.drawRoundRect(144f,226f,254f,254f,9f,9f,paint)
        paint.color=Color.WHITE; paint.textSize=16f; paint.typeface=Typeface.DEFAULT_BOLD; paint.textAlign=Paint.Align.CENTER
        val label = android.text.TextUtils.ellipsize(device,android.text.TextPaint(paint),104f,android.text.TextUtils.TruncateAt.END).toString()
        canvas.drawText(label,199f,246f,paint)
        val title = if(pane.source.isWindow) pane.source.application.ifBlank {pane.source.title} else device
        @Suppress("DEPRECATION")
        activity.setTaskDescription(ActivityManager.TaskDescription(title,icon,Color.rgb(24,52,59)))
    }
    fun restore(activity: Activity) {
        val drawable=activity.packageManager.getApplicationIcon(activity.applicationInfo)
        val icon=Bitmap.createBitmap(128,128,Bitmap.Config.ARGB_8888)
        drawable.setBounds(0,0,128,128);drawable.draw(Canvas(icon))
        @Suppress("DEPRECATION")
        activity.setTaskDescription(ActivityManager.TaskDescription("Weavelo", icon, Color.rgb(24,52,59)))
    }
}
