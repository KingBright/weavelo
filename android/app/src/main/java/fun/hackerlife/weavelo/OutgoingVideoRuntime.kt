package `fun`.hackerlife.weavelo

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update

internal data class OutgoingEncoderState(val id: Long = 0, val stream: Long = 0,
    val width: Int = 0, val height: Int = 0, val fps: Int = 0, val bitrateKbps: Int = 0)

internal object OutgoingVideoRuntime {
    private val mutable = MutableStateFlow(OutgoingEncoderState())
    val state = mutable.asStateFlow()
    fun report(value: OutgoingEncoderState) { if (value.id>0) mutable.value=value }
    fun clear(id: Long, stream: Long) {
        mutable.update { if (it.id==id && it.stream==stream) OutgoingEncoderState() else it }
    }
}
