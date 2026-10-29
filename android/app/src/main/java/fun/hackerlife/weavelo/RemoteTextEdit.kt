package `fun`.hackerlife.weavelo

/** The existing wire uses macOS physical key codes, including A at code zero. */
internal object RemoteTextEdit {
    data class Edit(val text: String, val start: Int, val end: Int)
    private val letters = mapOf(0x00 to 'a',0x0B to 'b',0x08 to 'c',0x02 to 'd',0x0E to 'e',
        0x03 to 'f',0x05 to 'g',0x04 to 'h',0x22 to 'i',0x26 to 'j',0x28 to 'k',0x25 to 'l',
        0x2E to 'm',0x2D to 'n',0x1F to 'o',0x23 to 'p',0x0C to 'q',0x0F to 'r',0x01 to 's',
        0x11 to 't',0x20 to 'u',0x09 to 'v',0x0D to 'w',0x07 to 'x',0x10 to 'y',0x06 to 'z')
    private val symbols = mapOf(0x12 to "1!",0x13 to "2@",0x14 to "3#",0x15 to "4$",0x17 to "5%",
        0x16 to "6^",0x1A to "7&",0x1C to "8*",0x19 to "9(",0x1D to "0)",0x18 to "=+",
        0x1B to "-_",0x1E to "]}",0x21 to "[{",0x27 to "'\"",0x29 to ";:",0x2A to "\\|",
        0x2B to ",<",0x2C to "/?",0x2F to ".>",0x31 to "  ",0x32 to "`~")
    fun edit(text: String, start: Int, end: Int, code: Int, modifiers: Int): Edit? {
        if (start !in 0..text.length || end !in 0..text.length) return null
        val low = minOf(start,end); val high = maxOf(start,end)
        val shift = modifiers and 1 != 0
        if (modifiers and 0x1E != 0) {
            return if (modifiers and 0x0A != 0 && modifiers and 0x14 == 0 && code == 0x00)
                Edit(text,0,text.length) else null
        }
        fun replace(from: Int, to: Int, value: String) = Edit(text.replaceRange(from,to,value),from+value.length,from+value.length)
        if (code == 0x33 || code == 0x75) {
            if (low != high) return replace(low,high,"")
            val from = if (code == 0x33 && low > 0) text.offsetByCodePoints(low,-1) else low
            val to = if (code == 0x75 && high < text.length) text.offsetByCodePoints(high,1) else high
            return replace(from,to,"")
        }
        val destination = when (code) {
            0x7B -> if (!shift && low != high) low else if (end > 0) text.offsetByCodePoints(end,-1) else end
            0x7C -> if (!shift && low != high) high else if (end < text.length) text.offsetByCodePoints(end,1) else end
            0x73 -> 0
            0x77 -> text.length
            else -> null
        }
        if (destination != null) return Edit(text,if (shift) start else destination,destination)
        val character = letters[code]?.let { if (shift xor (modifiers and 0x20 != 0)) it.uppercaseChar() else it }
            ?: symbols[code]?.get(if (shift) 1 else 0) ?: if (code == 0x24) '\n' else return null
        return replace(low,high,character.toString())
    }
}
