package com.sbchild.sb_edge;
import android.content.Context;
import android.os.Looper;
import java.lang.reflect.Method;

public class Main {
  public static void main(String[] args) {
    System.out.println("=== Into Java World ===");
    System.out.println("Got " + args.length + " params.");
    // for (int i = 0; i < args.length; i++) {
    //   System.out.println("arg[" + i + "]: " + args[i]);
    // }
    String jsonConfig = args.length > 0 ? args[0] : "{}";
    try {
      if (Looper.myLooper() == null) {
        System.out.println("Looper.myLooper() is null, creating a new one.");
        Looper.prepareMainLooper();
      }
      Class<?> activityThreadClass =
          Class.forName("android.app.ActivityThread");
      Method systemMainMethod = activityThreadClass.getMethod("systemMain");
      Object thread = systemMainMethod.invoke(null);
      Method getSystemContextMethod =
          activityThreadClass.getMethod("getSystemContext");
      Context context = (Context)getSystemContextMethod.invoke(thread);
      // target/aarch64-linux-android/release/libsb_edge.so
      String libPath = System.getenv("SBEDGE_LIBFILE_PATH");
      if (libPath != null) {
        System.out.println("Loading library with System.load from " + libPath);
        System.load(libPath);
      } else {
        System.out.println(
            "Loading library with System.loadLibrary from sb_edge");
        System.loadLibrary("sb_edge");
      }
      launchSbEdgeAndroid(context, jsonConfig);
      // no need to wait there: launchSbEdgeAndroid is blocking
      // System.out.println("Waiting for Looper.loop().");
      // Looper.loop();
      // System.out.println("Looper.loop() breaks.");
    } catch (Exception e) {
      e.printStackTrace();
    }
    System.out.println("End of Java World.");
    System.exit(0);
  }
  // Java_com_sbchild_sb_1edge_Main_launchSbEdgeAndroid
  private static native void launchSbEdgeAndroid(Object context,
                                                 String jsonConfig);
}
