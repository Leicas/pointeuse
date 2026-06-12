package __APP_PACKAGE__

import android.content.Intent
import android.os.Bundle
import androidx.activity.enableEdgeToEdge
import androidx.work.Configuration
import androidx.work.WorkManager
import app.tauri.Logger
import com.plugin.scheduletask.ScheduleTaskPlugin

class MainActivity : TauriActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
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
