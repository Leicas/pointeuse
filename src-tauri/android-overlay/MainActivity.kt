package __APP_PACKAGE__

import android.content.Intent
import android.os.Bundle
import androidx.activity.enableEdgeToEdge
import androidx.work.Configuration
import androidx.work.WorkManager
import app.tauri.Logger
import com.plugin.scheduletask.ScheduleTaskPlugin

class MainActivity : TauriActivity() {
    companion object {
        // tao's ndk_glue can only initialize once per process: a second
        // Activity creation in a warm process aborts with
        // "ndk-context assertion failed: previous.is_none()". When Android
        // hands us a recycled process that already hosted an activity,
        // relaunch into a fresh process instead of crashing.
        private var activityCreatedInThisProcess = false
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        if (activityCreatedInThisProcess) {
            Logger.warn("[MainActivity] Second activity creation in warm process — restarting process cleanly")
            val relaunch = packageManager.getLaunchIntentForPackage(packageName)?.apply {
                addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TASK)
            }
            if (relaunch != null) {
                val pi = android.app.PendingIntent.getActivity(
                    applicationContext, 0, relaunch,
                    android.app.PendingIntent.FLAG_IMMUTABLE or android.app.PendingIntent.FLAG_ONE_SHOT
                )
                val am = getSystemService(ALARM_SERVICE) as android.app.AlarmManager
                am.set(android.app.AlarmManager.RTC, System.currentTimeMillis() + 300, pi)
            }
            android.os.Process.killProcess(android.os.Process.myPid())
            return
        }
        activityCreatedInThisProcess = true

        enableEdgeToEdge()

        // Initialize WorkManager with our custom factory BEFORE super.onCreate()
        // (auto-init is disabled in the manifest)
        if (!WorkManager.isInitialized()) {
            val config = Configuration.Builder()
                .setWorkerFactory(AppWorkerFactory())
                .build()
            WorkManager.initialize(this, config)
            Logger.info("[MainActivity] WorkManager initialized with custom WorkerFactory")
        }

        super.onCreate(savedInstanceState)

        // Register notification action buttons (workaround for plugin storage bug)
        NotificationActionSetup.registerReminderActions(this)

        // Handle cold-start intent (WorkManager launched the activity while app was killed)
        handleScheduledTaskIntent(intent)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handleScheduledTaskIntent(intent)
    }

    private fun handleScheduledTaskIntent(intent: Intent?) {
        if (intent == null || !intent.hasExtra("run_task")) return

        Logger.info("[MainActivity] Forwarding scheduled task intent: ${intent.getStringExtra("run_task")}")
        ScheduleTaskPlugin.instance?.onNewIntent(intent)
            ?: Logger.error("[MainActivity] ScheduleTaskPlugin.instance is null — plugin not yet loaded")
    }
}
