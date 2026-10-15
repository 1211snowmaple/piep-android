package io.github.u1211snowmaple.piep.android

import android.os.Bundle
import android.webkit.WebView
import android.webkit.JavascriptInterface
import androidx.activity.OnBackPressedCallback
import androidx.activity.enableEdgeToEdge
import androidx.core.view.WindowInsetsControllerCompat

class MainActivity : TauriActivity() {
  override val handleBackNavigation: Boolean = false

  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
  }

  override fun onWebViewCreate(webView: WebView) {
    webView.addJavascriptInterface(object {
      @JavascriptInterface
      fun setDark(dark: Boolean) {
        runOnUiThread {
          val controller = WindowInsetsControllerCompat(window, window.decorView)
          controller.isAppearanceLightStatusBars = !dark
          controller.isAppearanceLightNavigationBars = !dark
        }
      }
    }, "piepAndroidAppearance")
    onBackPressedDispatcher.addCallback(this, object : OnBackPressedCallback(true) {
      override fun handleOnBackPressed() {
        webView.evaluateJavascript("window.piepAndroidBack?.() === true") { result ->
          if (result != "true") {
            isEnabled = false
            onBackPressedDispatcher.onBackPressed()
            isEnabled = true
          }
        }
      }
    })
  }
}
