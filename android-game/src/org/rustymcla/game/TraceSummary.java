package org.rustymcla.game;

import java.io.BufferedReader;
import java.io.IOException;
import java.io.Reader;

/** Keeps the native game thread ahead of unrelated Java service threads. */
final class TraceSummary {
    static String read(Reader input) throws IOException {
        BufferedReader reader = new BufferedReader(input);
        StringBuilder motor = new StringBuilder();
        StringBuilder main = new StringBuilder();
        StringBuilder other = new StringBuilder();
        StringBuilder section = new StringBuilder();
        String header = "";
        String line;
        int scanned = 0;
        while ((line = reader.readLine()) != null && scanned < 4 * 1024 * 1024) {
            scanned += line.length() + 1;
            if (line.startsWith("\"") || line.startsWith("----- end")) {
                collect(header, section, motor, main, other);
                section.setLength(0);
                header = line;
                if (line.startsWith("----- end")) break;
            }
            if (!header.isEmpty() && section.length() < 12000) {
                section.append(line, 0, Math.min(line.length(), 12000 - section.length())).append('\n');
            }
        }
        collect(header, section, motor, main, other);
        StringBuilder result = new StringBuilder();
        result.append(motor.length() == 0 ? "android_main thread not found in the available trace.\n" : "Game thread:\n" + motor);
        if (main.length() > 0) result.append("\nAndroid main thread:\n").append(main);
        if (other.length() > 0) result.append("\nOther relevant native threads:\n").append(other);
        return result.toString();
    }

    private static void collect(String header, StringBuilder section, StringBuilder motor,
                                StringBuilder main, StringBuilder other) {
        if (section.length() == 0) return;
        String value = section.toString();
        if (header.startsWith("\"android_main\"")) append(motor, value, 8000);
        else if (header.startsWith("\"main\"")) append(main, value, 3000);
        else if (value.contains("libmcla_android.so") || value.contains("vulkan") || value.contains("libGLES"))
            append(other, value, 3000);
    }

    private static void append(StringBuilder output, String value, int limit) {
        int room = limit - output.length();
        if (room > 0) output.append(value, 0, Math.min(value.length(), room));
    }
}
