package __APP_PACKAGE__

import android.content.Context
import androidx.work.ListenableWorker
import androidx.work.WorkerFactory
import androidx.work.WorkerParameters
import app.tauri.Logger

/**
 * Custom WorkerFactory that intercepts the schedule-task plugin's Worker
 * and replaces it with our override that calls the plugin directly.
 */
class AppWorkerFactory : WorkerFactory() {
    override fun createWorker(
        appContext: Context,
        workerClassName: String,
        workerParameters: WorkerParameters
    ): ListenableWorker? {
        return when (workerClassName) {
            "com.plugin.scheduletask.ScheduledTaskWorker" -> {
                Logger.info("[WorkerFactory] Intercepting ScheduledTaskWorker -> ScheduledTaskWorkerOverride")
                ScheduledTaskWorkerOverride(appContext, workerParameters)
            }
            else -> null // Let default factory handle other workers
        }
    }
}
