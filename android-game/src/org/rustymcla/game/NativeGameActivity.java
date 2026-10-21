package org.rustymcla.game;

import android.app.NativeActivity;
import android.content.Context;
import android.hardware.input.InputManager;
import android.os.Bundle;
import android.os.Handler;
import android.view.InputDevice;
import android.view.KeyEvent;
import android.view.MotionEvent;
import android.view.View;
import android.util.Log;
import java.util.HashSet;
import java.util.Set;

/** NativeActivity bridge for Android controller buttons, sticks, triggers, and D-pad. */
public final class NativeGameActivity extends NativeActivity
        implements InputManager.InputDeviceListener {
    // NativeActivity loads the same library for ANativeActivity_onCreate; this
    // explicit Java load also associates it with this class loader so the JNI
    // controller callbacks below can resolve by their exported symbol names.
    static { System.loadLibrary("mcla_android"); }

    private final Set<Integer> controllers = new HashSet<>();
    private final Set<Integer> axisLogged = new HashSet<>();
    private InputManager inputManager;

    private static native void nativePadConnected(int id);
    private static native void nativePadDisconnected(int id);
    private static native void nativePadAxis(int id, int axis, float value);
    private static native void nativePadButton(int id, int keyCode, boolean down);

    @Override public void onCreate(Bundle state) {
        super.onCreate(state);
        hideSystemBars();
        inputManager = (InputManager) getSystemService(Context.INPUT_SERVICE);
    }

    @Override protected void onResume() {
        super.onResume();
        hideSystemBars();
        if (inputManager == null) return;
        inputManager.registerInputDeviceListener(this, new Handler(getMainLooper()));
        for (int id : inputManager.getInputDeviceIds()) onInputDeviceAdded(id);
    }

    @Override protected void onPause() {
        if (inputManager != null) inputManager.unregisterInputDeviceListener(this);
        for (int id : new HashSet<>(controllers)) nativePadDisconnected(id);
        controllers.clear();
        super.onPause();
    }

    @Override public void onWindowFocusChanged(boolean hasFocus) {
        super.onWindowFocusChanged(hasFocus);
        if (hasFocus) hideSystemBars();
    }

    private void hideSystemBars() {
        getWindow().getDecorView().setSystemUiVisibility(
            View.SYSTEM_UI_FLAG_FULLSCREEN
                | View.SYSTEM_UI_FLAG_HIDE_NAVIGATION
                | View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY
                | View.SYSTEM_UI_FLAG_LAYOUT_STABLE
                | View.SYSTEM_UI_FLAG_LAYOUT_FULLSCREEN
                | View.SYSTEM_UI_FLAG_LAYOUT_HIDE_NAVIGATION
        );
    }

    private boolean isController(int id) {
        InputDevice device = InputDevice.getDevice(id);
        if (device == null || device.isVirtual()) return false;
        int sources = device.getSources();
        return (sources & InputDevice.SOURCE_GAMEPAD) == InputDevice.SOURCE_GAMEPAD
                || (sources & InputDevice.SOURCE_JOYSTICK) == InputDevice.SOURCE_JOYSTICK
                || (sources & InputDevice.SOURCE_DPAD) == InputDevice.SOURCE_DPAD;
    }

    @Override public void onInputDeviceAdded(int id) {
        if (isController(id) && controllers.add(id)) {
            InputDevice device = InputDevice.getDevice(id);
            Log.i("RustyPad", "connected id=" + id + " name=" + (device == null ? "?" : device.getName()));
            nativePadConnected(id);
        }
    }

    @Override public void onInputDeviceRemoved(int id) {
        if (controllers.remove(id)) nativePadDisconnected(id);
    }

    @Override public void onInputDeviceChanged(int id) {
        if (!isController(id)) onInputDeviceRemoved(id);
        else onInputDeviceAdded(id);
    }

    @Override public boolean dispatchKeyEvent(KeyEvent event) {
        int id = event.getDeviceId();
        int code = event.getKeyCode();
        if (isController(id) && isPadKey(code)) {
            if (event.getAction() == KeyEvent.ACTION_DOWN || event.getAction() == KeyEvent.ACTION_UP) {
                onInputDeviceAdded(id);
                if (event.getRepeatCount() == 0 || event.getAction() == KeyEvent.ACTION_UP) {
                    Log.i("RustyPad", "button id=" + id + " key=" + code + " down=" + (event.getAction() == KeyEvent.ACTION_DOWN));
                    nativePadButton(id, code, event.getAction() == KeyEvent.ACTION_DOWN);
                }
                return true;
            }
        }
        return super.dispatchKeyEvent(event);
    }

    private static boolean isPadKey(int key) {
        switch (key) {
            case KeyEvent.KEYCODE_DPAD_UP: case KeyEvent.KEYCODE_DPAD_DOWN:
            case KeyEvent.KEYCODE_DPAD_LEFT: case KeyEvent.KEYCODE_DPAD_RIGHT:
            case KeyEvent.KEYCODE_BUTTON_A: case KeyEvent.KEYCODE_BUTTON_B:
            case KeyEvent.KEYCODE_BUTTON_X: case KeyEvent.KEYCODE_BUTTON_Y:
            case KeyEvent.KEYCODE_BUTTON_L1: case KeyEvent.KEYCODE_BUTTON_R1:
            case KeyEvent.KEYCODE_BUTTON_L2: case KeyEvent.KEYCODE_BUTTON_R2:
            case KeyEvent.KEYCODE_BUTTON_THUMBL: case KeyEvent.KEYCODE_BUTTON_THUMBR:
            case KeyEvent.KEYCODE_BUTTON_START: case KeyEvent.KEYCODE_BUTTON_SELECT:
                return true;
            default: return false;
        }
    }

    @Override public boolean dispatchGenericMotionEvent(MotionEvent event) {
        int id = event.getDeviceId();
        if (isController(id) && (event.getSource() & InputDevice.SOURCE_JOYSTICK) == InputDevice.SOURCE_JOYSTICK
                && event.getAction() == MotionEvent.ACTION_MOVE) {
            onInputDeviceAdded(id);
            InputDevice device = event.getDevice();
            if (device != null) {
                sendAxisIfPresent(event, device, MotionEvent.AXIS_X);
                sendAxisIfPresent(event, device, MotionEvent.AXIS_Y);
                sendAxisIfPresent(event, device, MotionEvent.AXIS_Z);
                sendAxisIfPresent(event, device, MotionEvent.AXIS_RZ);
                // Some Android HID drivers expose a second stick on RX/RY rather
                // than the standardized Z/RZ pair. Send both; Rust picks the active one.
                sendAxisIfPresent(event, device, MotionEvent.AXIS_RX);
                sendAxisIfPresent(event, device, MotionEvent.AXIS_RY);
                sendAxisIfPresent(event, device, MotionEvent.AXIS_LTRIGGER);
                sendAxisIfPresent(event, device, MotionEvent.AXIS_RTRIGGER);
                sendAxisIfPresent(event, device, MotionEvent.AXIS_BRAKE);
                sendAxisIfPresent(event, device, MotionEvent.AXIS_GAS);
                sendAxisIfPresent(event, device, MotionEvent.AXIS_HAT_X);
                sendAxisIfPresent(event, device, MotionEvent.AXIS_HAT_Y);
                if (axisLogged.add(id)) Log.i("RustyPad", "joystick axes received id=" + id + " name=" + device.getName());
            }
            return true;
        }
        return super.dispatchGenericMotionEvent(event);
    }

    private void sendAxisIfPresent(MotionEvent event, InputDevice device, int axis) {
        if (device.getMotionRange(axis, event.getSource()) != null)
            nativePadAxis(event.getDeviceId(), axis, event.getAxisValue(axis));
    }
}
