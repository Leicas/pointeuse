package __APP_PACKAGE__

import android.content.Intent
import android.os.Bundle
import androidx.activity.enableEdgeToEdge
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

        // WorkManager is initialized on demand by PointeuseApplication
        // (Configuration.Provider) so that it also works when Android starts
        // this process for SystemJobService alone, with no activity.

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

    override fun onDestroy() {
        super.onDestroy()
        // tao's ndk_glue can only initialize once per process, so once this
        // activity is gone the cached process can never host another one —
        // Android relaunching into it would hit the warm-process guard above,
        // which the OS surfaces as an "app keeps stopping" crash dialog.
        // With no live activity, exiting here is an ordinary background
        // process death: the next launch is a clean cold start instead.
        // Skip config-change recreation, where a new activity follows
        // immediately in this same process.
        if (!isChangingConfigurations) {
            Logger.info("[MainActivity] Activity destroyed — exiting process so the next launch cold-starts")
            android.os.Process.killProcess(android.os.Process.myPid())
        }
    }

    private fun handleScheduledTaskIntent(intent: Intent?) {
        if (intent == null || !intent.hasExtra("run_task")) return

        Logger.info("[MainActivity] Forwarding scheduled task intent: ${intent.getStringExtra("run_task")}")
        ScheduleTaskPlugin.instance?.onNewIntent(intent)
            ?: Logger.error("[MainActivity] ScheduleTaskPlugin.instance is null — plugin not yet loaded")
    }
}
