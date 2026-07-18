package dev.lorepia.spike.keychain

import android.content.Context
import android.os.Bundle
import androidx.activity.enableEdgeToEdge

class MainActivity : TauriActivity() {
  private external fun initNdkContext(context: Context)

  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
    initNdkContext(applicationContext)
  }
}
