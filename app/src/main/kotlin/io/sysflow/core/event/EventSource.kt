package io.sysflow.core.event

import io.sysflow.core.model.ModuleEventPage
import io.sysflow.core.root.SysFlowClient

/**
 * Read port over the root-owned event journal.
 *
 * Kept separate from [SysFlowClient] so the Room sync logic can be tested without root/libsu.
 */
interface EventSource {
    suspend fun events(
        limit: Int = 200,
        afterTimestampMs: Long? = null,
        afterId: String? = null,
        beforeTimestampMs: Long? = null,
        beforeId: String? = null,
        ascending: Boolean = false,
    ): ModuleEventPage
}

/** Adapts the real root client to [EventSource]. */
class SysFlowClientEventSource(private val client: SysFlowClient) : EventSource {
    override suspend fun events(
        limit: Int,
        afterTimestampMs: Long?,
        afterId: String?,
        beforeTimestampMs: Long?,
        beforeId: String?,
        ascending: Boolean,
    ): ModuleEventPage = client.events(
        limit = limit,
        afterTimestampMs = afterTimestampMs,
        afterId = afterId,
        beforeTimestampMs = beforeTimestampMs,
        beforeId = beforeId,
        ascending = ascending,
    )
}
