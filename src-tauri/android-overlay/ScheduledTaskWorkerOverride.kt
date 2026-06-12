package __APP_PACKAGE__

import android.content.Context
import android.content.Intent
import androidx.work.Worker
import androidx.work.WorkerParameters
import app.tauri.Logger
import app.tauri.plugin.JSObject
import com.plugin.scheduletask.ScheduleTaskPlugin

/**
 * Replaces the plugin's ScheduledTaskWorker via WorkerFactory.
 * Instead of calling startActivity() (which fails to trigger onNewIntent),
 * we call the plugin's channel directly when the app is alive.
 */
class ScheduledTaskWorkerOverride(context: Context, params: WorkerParameters) : Worker(context, params) {

    override fun doWork(): Result {
        val taskId = inputData.getString("taskId") ?: return Result.failure()
        val taskName = inputData.getString("taskName") ?: return Result.failure()
        val packageName = inputData.getString("packageName") ?: return Result.failure()

        Logger.info("[WorkerOverride] Task fired: $taskName ($taskId)")

        // Collect parameters
        val params = mutableMapOf<String, String>()
        for (key in inputData.keyValueMap.keys) {
            if (key.startsWith("param_")) {
                val paramName = key.removePrefix("param_")
                val paramValue = inputData.getString(key) ?: continue
                params[paramName] = paramValue
            }
        }

        // Try direct plugin call first (works when app process is alive)
        val plugin = ScheduleTaskPlugin.instance
        if (plugin != null) {
            Logger.info("[WorkerOverride] Plugin alive, calling onNewIntent directly")
            val intent = Intent().apply {
                putExtra("run_task", taskName)
                putExtra("task_id", taskId)
                params.forEach { (k, v) -> putExtra("task_param_$k", v) }
            }
            plugin.onNewIntent(intent)
            return Result.success()
        }

        // Fallback: launch activity (app was killed, WorkManager restarted process)
        Logger.info("[WorkerOverride] Plugin not initialized, launching MainActivity")
        return try {
            val intent = Intent().apply {
                setClassName(packageName, "$packageName.MainActivity")
                addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP)
                putExtra("run_task", taskName)
                putExtra("task_id", taskId)
                params.forEach { (k, v) -> putExtra("task_param_$k", v) }
            }
            applicationContext.startActivity(intent)
            Result.success()
        } catch (e: Exception) {
            Logger.error("[WorkerOverride] Failed to start activity: ${e.message}")
            Result.failure()
        }
    }
}
