package org.rustymcla.game;

import android.app.Activity;
import android.app.ActivityManager;
import android.app.ApplicationExitInfo;
import android.content.ClipData;
import android.content.ClipboardManager;
import android.content.Intent;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.os.Environment;
import android.provider.Settings;
import android.widget.Button;
import android.widget.ImageView;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;
import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.io.PrintWriter;
import java.io.RandomAccessFile;
import java.util.List;

/** Launches the native engine directly from the user's shared Rusty folder. */
public final class MainActivity extends Activity {
    private TextView status;
    private Button permission;
    private Button play;
    private TextView diagnostic;
    private String diagnosticText = "";

    @Override public void onCreate(Bundle state) {
        super.onCreate(state);
        installCrashRecorder();
        LinearLayout panel = new LinearLayout(this);
        panel.setOrientation(LinearLayout.VERTICAL);
        int pad = (int) (20 * getResources().getDisplayMetrics().density);
        panel.setPadding(pad, pad, pad, pad);
        ImageView cover = new ImageView(this);
        cover.setImageResource(getResources().getIdentifier("cover", "drawable", getPackageName()));
        cover.setAdjustViewBounds(true);
        cover.setMaxHeight((int) (300 * getResources().getDisplayMetrics().density));
        panel.addView(cover);
        TextView title = new TextView(this);
        title.setText("Rusty MCLA — Android experimental");
        title.setTextSize(23);
        panel.addView(title);
        TextView explanation = new TextView(this);
        explanation.setText("Extraia todas as 154 partes ZIP na pasta Rusty da memória interna, ao lado de Download. Deve existir Rusty/RustyMCLA/manifest.local.json e Rusty/RustyMCLA/data/cache. O jogo lerá esses arquivos diretamente, sem copiar 13,5 GB para dentro do aplicativo. A primeira abertura pode levar algum tempo.");
        explanation.setTextSize(16);
        panel.addView(explanation);
        permission = new Button(this);
        permission.setText("Permitir acesso aos arquivos");
        permission.setOnClickListener(v -> requestFilesAccess());
        panel.addView(permission);
        play = new Button(this);
        play.setText("Preparar e iniciar motor");
        play.setOnClickListener(v -> prepareAndPlay());
        panel.addView(play);
        status = new TextView(this);
        status.setTextSize(16);
        panel.addView(status);
        Button copyDiagnostic = new Button(this);
        copyDiagnostic.setText("Copiar diagnóstico");
        copyDiagnostic.setOnClickListener(v -> {
            ClipboardManager clipboard = (ClipboardManager) getSystemService(CLIPBOARD_SERVICE);
            clipboard.setPrimaryClip(ClipData.newPlainText("Rusty MCLA diagnóstico", diagnosticText));
            status.setText("Diagnóstico copiado. Cole o texto na conversa.");
        });
        panel.addView(copyDiagnostic);
        diagnostic = new TextView(this);
        diagnostic.setTextIsSelectable(true);
        panel.addView(diagnostic);
        ScrollView scroll = new ScrollView(this);
        scroll.addView(panel);
        setContentView(scroll);
        refresh();
    }

    private void installCrashRecorder() {
        Thread.UncaughtExceptionHandler previous = Thread.getDefaultUncaughtExceptionHandler();
        Thread.setDefaultUncaughtExceptionHandler((thread, error) -> {
            try (PrintWriter output = new PrintWriter(new FileOutputStream(
                    new File(getFilesDir(), "last-java-crash.txt")))) {
                output.println("Thread: " + thread.getName());
                error.printStackTrace(output);
            } catch (Throwable ignored) { }
            if (previous != null) previous.uncaughtException(thread, error);
        });
    }

    @Override protected void onResume() {
        super.onResume();
        if (status != null) refresh();
    }

    private boolean hasFilesAccess() {
        return Build.VERSION.SDK_INT < 30 || Environment.isExternalStorageManager();
    }

    private void requestFilesAccess() {
        if (Build.VERSION.SDK_INT < 30) { refresh(); return; }
        try {
            startActivity(new Intent(Settings.ACTION_MANAGE_APP_ALL_FILES_ACCESS_PERMISSION,
                Uri.parse("package:" + getPackageName())));
        } catch (Exception error) {
            startActivity(new Intent(Settings.ACTION_MANAGE_ALL_FILES_ACCESS_PERMISSION));
        }
    }

    private File sharedRoot() {
        return new File(Environment.getExternalStorageDirectory(), "Rusty");
    }

    private File installRoot() {
        File base = sharedRoot();
        if (new File(base, "manifest.local.json").isFile()) return base;
        File nested = new File(base, "RustyMCLA");
        if (new File(nested, "manifest.local.json").isFile()) return nested;
        return null;
    }

    private boolean ready(File root) {
        return root != null && new File(root, "data/cache/resources").isDirectory()
            && new File(root, "data/cache/tune").isDirectory()
            && new File(root, "data/cache/shaders_wgsl").isDirectory();
    }

    private void refresh() {
        boolean granted = hasFilesAccess();
        permission.setVisibility(granted ? Button.GONE : Button.VISIBLE);
        if (granted && !sharedRoot().exists()) sharedRoot().mkdirs();
        boolean found = granted && ready(installRoot());
        play.setEnabled(found);
        status.setText(!granted ? "Autorize o acesso a todos os arquivos para ler a pasta Rusty na memória interna."
            : found ? "Dados encontrados em " + installRoot().getAbsolutePath() + ". Pode iniciar."
            : "Pasta ainda não encontrada. Crie Rusty na raiz da memória interna e extraia ali todos os ZIPs. Procuro Rusty/RustyMCLA/manifest.local.json e Rusty/RustyMCLA/data/cache.");
        diagnosticText = buildDiagnostic();
        diagnostic.setText(diagnosticText);
    }

    private void prepareAndPlay() {
        File root = installRoot();
        if (!hasFilesAccess() || !ready(root)) { refresh(); return; }
        play.setEnabled(false);
        status.setText("Preparando os dados. Mantenha o aplicativo aberto.");
        new Thread(() -> {
            try {
                installWorldIndex(root);
                joinIntroVideo(root);
                runOnUiThread(() -> {
                    refresh();
                    getSharedPreferences("diagnostics", MODE_PRIVATE).edit()
                        .putLong("last_native_launch", System.currentTimeMillis()).apply();
                    startActivity(new Intent().setClassName(getPackageName(), "android.app.NativeActivity"));
                });
            } catch (Exception error) {
                runOnUiThread(() -> {
                    refresh();
                    status.setText("Preparação falhou: " + error.getMessage());
                });
            }
        }, "mcla-prepare").start();
    }

    private String buildDiagnostic() {
        StringBuilder text = new StringBuilder("Diagnóstico Rusty MCLA 0.7\n");
        text.append("Android: ").append(Build.VERSION.RELEASE).append("; aparelho: ")
            .append(Build.MANUFACTURER).append(' ').append(Build.MODEL).append('\n');
        text.append("Acesso aos arquivos: ").append(hasFilesAccess()).append('\n');
        File root = installRoot();
        text.append("Pasta: ").append(root == null ? "não encontrada" : root.getAbsolutePath()).append('\n');
        File index = root == null ? null : new File(root, "data/cache/world_index.json");
        text.append("Índice do mundo: ").append(index != null && index.isFile() ? index.length() + " bytes" : "ausente").append('\n');
        long started = getSharedPreferences("diagnostics", MODE_PRIVATE).getLong("last_native_launch", 0);
        if (Build.VERSION.SDK_INT >= 30 && started != 0) {
            try {
                ActivityManager manager = (ActivityManager) getSystemService(ACTIVITY_SERVICE);
                List<ApplicationExitInfo> exits = manager.getHistoricalProcessExitReasons(getPackageName(), 0, 5);
                int shown = 0;
                for (ApplicationExitInfo exit : exits) {
                    if (exit.getTimestamp() >= started - 1000) {
                        text.append("Saída ").append(++shown).append(": motivo=").append(exit.getReason())
                            .append(", status=").append(exit.getStatus())
                            .append(", descrição=").append(exit.getDescription()).append('\n');
                        if (exit.getReason() == ApplicationExitInfo.REASON_ANR) {
                            try (InputStream trace = exit.getTraceInputStream()) {
                                if (trace != null) {
                                    text.append("Rastreamento ANR (início):\n");
                                    byte[] bytes = new byte[20480];
                                    int count = 0;
                                    while (count < bytes.length) {
                                        int read = trace.read(bytes, count, bytes.length - count);
                                        if (read <= 0) break;
                                        count += read;
                                    }
                                    text.append(new String(bytes, 0, count, java.nio.charset.StandardCharsets.UTF_8)).append('\n');
                                }
                            } catch (Exception error) {
                                text.append("Rastreamento ANR indisponível: ").append(error).append('\n');
                            }
                        }
                        if (shown >= 3) break;
                    }
                }
            } catch (Exception error) {
                text.append("Histórico de saída indisponível: ").append(error).append('\n');
            }
        }
        File javaCrash = new File(getFilesDir(), "last-java-crash.txt");
        if (javaCrash.isFile()) {
            text.append("\nExceção Java:\n");
            appendFileTail(text, javaCrash);
        }
        File logs = new File(getFilesDir(), "user/logs");
        for (String prefix : new String[] {"crash-", "game-"}) {
            File latest = null;
            File[] files = logs.listFiles();
            if (files != null) for (File file : files) {
                if (file.getName().startsWith(prefix) && (latest == null || file.lastModified() > latest.lastModified()))
                    latest = file;
            }
            if (latest != null) {
                text.append("\n").append(latest.getName()).append(":\n");
                appendFileTail(text, latest);
            }
        }
        return text.toString();
    }

    private static void appendFileTail(StringBuilder text, File file) {
        try (RandomAccessFile input = new RandomAccessFile(file, "r")) {
            long length = input.length();
            input.seek(Math.max(0, length - 12000));
            byte[] bytes = new byte[(int) (length - input.getFilePointer())];
            input.readFully(bytes);
            text.append(new String(bytes, java.nio.charset.StandardCharsets.UTF_8));
        } catch (Exception error) {
            text.append("Não consegui ler: ").append(error).append('\n');
        }
    }

    private void installWorldIndex(File root) throws Exception {
        File index = new File(root, "data/cache/world_index.json");
        File temp = new File(index.getPath() + ".tmp");
        try (InputStream in = getAssets().open("world_index.json");
             FileOutputStream out = new FileOutputStream(temp)) {
            byte[] bytes = new byte[8192];
            int count;
            while ((count = in.read(bytes)) != -1) out.write(bytes, 0, count);
        }
        if (!temp.renameTo(index)) throw new IllegalStateException("Não foi possível gravar o índice do mundo.");
    }

    private static void joinIntroVideo(File root) throws Exception {
        File video = new File(root, "data/video_pc/intro.mcv");
        if (video.isFile()) return;
        File first = new File(video.getPath() + ".part01");
        File second = new File(video.getPath() + ".part02");
        if (!first.isFile() || !second.isFile())
            throw new IllegalArgumentException("Faltam as partes 153 e 154 do vídeo de abertura.");
        File temp = new File(video.getPath() + ".tmp");
        byte[] bytes = new byte[1024 * 1024];
        try (FileOutputStream out = new FileOutputStream(temp)) {
            for (File part : new File[] {first, second}) {
                try (FileInputStream in = new FileInputStream(part)) {
                    int count;
                    while ((count = in.read(bytes)) != -1) out.write(bytes, 0, count);
                }
            }
        }
        if (!temp.renameTo(video)) throw new IllegalStateException("Não foi possível finalizar intro.mcv.");
        if (!first.delete() || !second.delete()) throw new IllegalStateException("Não foi possível limpar partes do vídeo.");
    }
}
