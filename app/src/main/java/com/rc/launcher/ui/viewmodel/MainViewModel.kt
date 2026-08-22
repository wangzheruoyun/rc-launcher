package com.rc.launcher.ui.viewmodel

import androidx.lifecycle.ViewModel
import androidx.lifecycle.viewModelScope
import com.rc.launcher.core.RustBridge
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch

/**
 * Immutable UI state for the home dashboard (task 11: "ViewModel / StateFlow
 * state containers"). A single [StateFlow] of a sealed hierarchy keeps the UI
 * exhaustive and free of partial/null states.
 */
sealed interface MainUiState {
    data object Loading : MainUiState
    data class Ready(val coreVersion: String, val greeting: String) : MainUiState
    data class Error(val message: String) : MainUiState
}

/**
 * App-level state container.
 *
 * It moves the Rust-core probes out of the composition (the previous
 * [com.rc.launcher.MainActivity] called [RustBridge.getVersion] / [RustBridge.greet]
 * directly on every recomposition) into a [ViewModel] and exposes an immutable
 * [MainUiState] [StateFlow]. The native calls are wrapped so a missing/failed
 * core library degrades gracefully instead of crashing the UI (task 19).
 */
class MainViewModel : ViewModel() {
    private val _uiState = MutableStateFlow<MainUiState>(MainUiState.Loading)
    val uiState: StateFlow<MainUiState> = _uiState.asStateFlow()

    init {
        loadCoreInfo()
    }

    private fun loadCoreInfo() {
        viewModelScope.launch(Dispatchers.IO) {
            runCatching {
                val version = RustBridge.getVersion()
                val greeting = RustBridge.greet("Player")
                _uiState.value = MainUiState.Ready(version, greeting)
            }.onFailure { error ->
                _uiState.value = MainUiState.Error(
                    error.message ?: (error::class.simpleName ?: "unknown"),
                )
            }
        }
    }

    /** Re-run the core probe (e.g. after the library becomes available). */
    fun retry() {
        _uiState.value = MainUiState.Loading
        loadCoreInfo()
    }
}
