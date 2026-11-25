package io.sysflow.core.root

import com.topjohnwu.superuser.Shell
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
import kotlin.coroutines.resume

data class ShellResult(
    val stdout: String,
    val stderr: String,
    val exitCode: Int,
) {
    val isSuccess: Boolean get() = exitCode == 0
}

class ShellTimeoutException(timeoutMs: Long) : Exception("命令执行超时（${timeoutMs}ms）")

/**
 * One persistent libsu root shell for the whole process. Every SysFlow CLI call is a single
 * command in that shell, so polling does not fork a new `su` each time.
 */
class RootExecutor {

    suspend fun hasAccess(): Boolean = withContext(Dispatchers.IO) {
        try {
            val shell = Shell.getShell()
            if (shell.isRoot) {
                true
            } else {
                // libsu caches the non-root shell; close it so the next attempt asks for root again.
                shell.close()
                false
            }
        } catch (_: Exception) {
            false
        }
    }

    suspend fun exec(command: String, timeoutMs: Long = DEFAULT_TIMEOUT_MS): ShellResult {
        val result = withTimeoutOrNull(timeoutMs) {
            suspendCancellableCoroutine { continuation ->
                val stdout = ArrayList<String>()
                val stderr = ArrayList<String>()
                Shell.cmd(command).to(stdout, stderr).submit { shellResult ->
                    if (continuation.isActive) {
                        continuation.resume(
                            ShellResult(
                                stdout = stdout.joinToString("\n"),
                                stderr = stderr.joinToString("\n"),
                                exitCode = shellResult.code,
                            ),
                        )
                    }
                }
            }
        }
        return result ?: throw ShellTimeoutException(timeoutMs)
    }

    companion object {
        const val DEFAULT_TIMEOUT_MS: Long = 15_000
    }
}
