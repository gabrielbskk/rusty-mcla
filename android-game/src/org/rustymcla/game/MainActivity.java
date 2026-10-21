package org.rustymcla.game;

import android.app.Activity;
import android.content.Intent;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.os.Environment;
import android.provider.Settings;
import android.widget.Button;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;
import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;

/** Launches the native engine directly from the user's shared Rusty folder. */
public final class MainActivity extends Activity {
    private TextView status;
    private Button permission;
    private Button play;

    @Override public void onCreate(Bundle state) {
        super.onCreate(state);
        LinearLayout panel = new LinearLayout(this);
        panel.setOrientation(LinearLayout.VERTICAL);
        int pad = (int) (20 * getResources().getDisplayMetrics().density);
        panel.setPadding(pad, pad, pad, pad);
        TextView title = new TextView(this);
        title.setText("Rusty MCLA — Android experimental");
        title.setTextSize(23);
        panel.addView(title);
        TextView explanation = new TextView(this);
        explanation.setText("Extraia todas as 154 partes ZIP na pasta Rusty da memória interna, ao lado de Download. Deve existir Rusty/RustyMCLA/manifest.local.json e Rusty/RustyMCLA/data/cache. O jogo lerá esses arquivos diretamente, sem copiar 13,5 GB para dentro do aplicativo. O motor ainda não foi testado num aparelho.");
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
        ScrollView scroll = new ScrollView(this);
        scroll.addView(panel);
        setContentView(scroll);
        refresh();
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
    }

    private void prepareAndPlay() {
        File root = installRoot();
        if (!hasFilesAccess() || !ready(root)) { refresh(); return; }
        play.setEnabled(false);
        status.setText("Preparando vídeo de abertura. Mantenha o aplicativo aberto.");
        new Thread(() -> {
            try {
                joinIntroVideo(root);
                runOnUiThread(() -> {
                    refresh();
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
