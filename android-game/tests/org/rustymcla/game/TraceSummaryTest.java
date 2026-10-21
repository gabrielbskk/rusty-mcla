package org.rustymcla.game;
import java.io.StringReader;
public final class TraceSummaryTest {
    public static void main(String[] args) throws Exception {
        StringBuilder trace = new StringBuilder("header\n\"main\"\n main_wait\n");
        for (int i = 0; i < 2000; i++) trace.append("\"worker").append(i).append("\"\n unrelated service\n");
        trace.append("\"android_main\"\n libmcla_android.so native_block\n----- end 123 -----\n\"android_main\"\n wrong_process\n");
        String result = TraceSummary.read(new StringReader(trace.toString()));
        if (!result.contains("native_block") || result.contains("wrong_process") || result.indexOf("native_block") > result.indexOf("main_wait")) throw new AssertionError(result);
        if (!TraceSummary.read(new StringReader("no threads")).contains("not found")) throw new AssertionError("Missing-thread notice");
        StringBuilder huge = new StringBuilder("\"android_main\"\n");
        for (int i=0;i<30000;i++) huge.append("long native stack line\n");
        if (TraceSummary.read(new StringReader(huge.toString())).length() > 8100) throw new AssertionError("Unbounded output");
        System.out.println("TraceSummary: 3 tests passed");
    }
}
