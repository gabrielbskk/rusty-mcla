package org.rustymcla.game;

import android.app.Activity;
import android.content.Intent;
import android.database.Cursor;
import android.net.Uri;
import android.os.Bundle;
import android.provider.DocumentsContract;
import android.widget.Button;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;
import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.InputStream;

/** Imports data prepared by mcla-tool and launches the Rust NativeActivity. */
public final class MainActivity extends Activity {
    private static final int PICK_FOLDER = 1;
    private TextView status;
    private Button play;
    private Button importButton;

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
        explanation.setText("Este APK contém o motor ARM64. Ele precisa dos dados convertidos do seu jogo. As partes ZIP ou a ISO do Xbox não servem aqui diretamente. O port Android ainda não foi validado num aparelho e pode falhar ao abrir.");
        explanation.setTextSize(16);
        panel.addView(explanation);
        importButton = new Button(this);
        importButton.setText("Importar pasta RustyMCLA convertida");
        importButton.setOnClickListener(v -> {
            Intent pick = new Intent(Intent.ACTION_OPEN_DOCUMENT_TREE);
            pick.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
            startActivityForResult(pick, PICK_FOLDER);
        });
        panel.addView(importButton);
        play = new Button(this);
        play.setText("Iniciar motor");
        play.setOnClickListener(v -> startActivity(new Intent().setClassName(getPackageName(), "android.app.NativeActivity")));
        panel.addView(play);
        status = new TextView(this);
        status.setTextSize(16);
        panel.addView(status);
        ScrollView scroll = new ScrollView(this);
        scroll.addView(panel);
        setContentView(scroll);
        refresh();
    }

    private boolean ready() {
        File root = getFilesDir();
        return new File(root, "manifest.local.json").isFile()
            && new File(root, "data/cache/resources").isDirectory()
            && new File(root, "data/cache/tune").isDirectory()
            && new File(root, "data/cache/shaders_wgsl").isDirectory();
    }

    private void refresh() {
        boolean ok = ready();
        play.setEnabled(ok);
        status.setText(ok ? "Dados básicos encontrados. Você pode tentar iniciar o motor."
            : "Nenhuma instalação convertida encontrada. Sem os dados, o motor não inicia. A conversão dos arquivos do Drive ainda é necessária.");
    }

    @Override protected void onActivityResult(int request, int result, Intent data) {
        super.onActivityResult(request, result, data);
        if (request != PICK_FOLDER || result != RESULT_OK || data == null || data.getData() == null) return;
        Uri tree = data.getData();
        importButton.setEnabled(false);
        play.setEnabled(false);
        status.setText("Importando dados. Isso pode demorar e ocupar mais de 14 GB. Mantenha o app aberto.");
        new Thread(() -> {
            try {
                Uri root = DocumentsContract.buildDocumentUriUsingTree(tree, DocumentsContract.getTreeDocumentId(tree));
                Uri manifest = child(tree, root, "manifest.local.json");
                Uri gameData = child(tree, root, "data");
                if (manifest == null || gameData == null || child(tree, gameData, "cache") == null)
                    throw new IllegalArgumentException("Selecione a pasta de saída RustyMCLA, com manifest.local.json e data/cache.");
                File staging = new File(getFilesDir(), "importing");
                deleteTree(staging);
                if (!staging.mkdirs()) throw new IllegalStateException("Não foi possível criar pasta temporária.");
                copy(tree, gameData, new File(staging, "data"), 0);
                copy(tree, manifest, new File(staging, "manifest.local.json"), 0);
                joinIntroVideo(staging);
                File oldData = new File(getFilesDir(), "data");
                deleteTree(oldData);
                if (!new File(staging, "data").renameTo(oldData)) throw new IllegalStateException("Não foi possível mover os dados.");
                File oldManifest = new File(getFilesDir(), "manifest.local.json");
                if (oldManifest.exists() && !oldManifest.delete()) throw new IllegalStateException("Não foi possível substituir o manifesto.");
                if (!new File(staging, "manifest.local.json").renameTo(oldManifest)) throw new IllegalStateException("Não foi possível mover o manifesto.");
                deleteTree(staging);
                runOnUiThread(() -> { importButton.setEnabled(true); refresh(); });
            } catch (Exception error) {
                runOnUiThread(() -> { importButton.setEnabled(true); refresh(); status.setText("Importação falhou: " + error.getMessage()); });
            }
        }, "mcla-import").start();
    }

    private static void joinIntroVideo(File staging) throws Exception {
        File video = new File(staging, "data/video_pc/intro.mcv");
        File first = new File(video.getPath() + ".part01");
        File second = new File(video.getPath() + ".part02");
        if (!first.exists() && !second.exists()) return;
        if (!first.isFile() || !second.isFile()) throw new IllegalArgumentException("Falta uma parte do vídeo de abertura.");
        byte[] bytes = new byte[1024 * 1024];
        try (FileOutputStream out = new FileOutputStream(video)) {
            for (File part : new File[] {first, second}) {
                try (FileInputStream in = new FileInputStream(part)) {
                    int count;
                    while ((count = in.read(bytes)) != -1) out.write(bytes, 0, count);
                }
            }
        }
        if (!first.delete() || !second.delete()) throw new IllegalStateException("Não foi possível limpar partes do vídeo.");
    }

    private Uri child(Uri tree, Uri parent, String name) {
        Uri children = DocumentsContract.buildChildDocumentsUriUsingTree(tree, DocumentsContract.getDocumentId(parent));
        String[] fields = {DocumentsContract.Document.COLUMN_DOCUMENT_ID, DocumentsContract.Document.COLUMN_DISPLAY_NAME};
        try (Cursor c = getContentResolver().query(children, fields, null, null, null)) {
            if (c != null) while (c.moveToNext()) {
                if (name.equals(c.getString(1))) return DocumentsContract.buildDocumentUriUsingTree(tree, c.getString(0));
            }
        }
        return null;
    }

    private void copy(Uri tree, Uri source, File destination, int depth) throws Exception {
        if (depth > 64) throw new IllegalArgumentException("Pastas aninhadas demais.");
        String[] fields = {DocumentsContract.Document.COLUMN_MIME_TYPE};
        String type;
        try (Cursor c = getContentResolver().query(source, fields, null, null, null)) {
            if (c == null || !c.moveToFirst()) throw new IllegalArgumentException("Arquivo inacessível: " + source);
            type = c.getString(0);
        }
        if (DocumentsContract.Document.MIME_TYPE_DIR.equals(type)) {
            if (!destination.isDirectory() && !destination.mkdirs()) throw new IllegalStateException("Sem espaço ou permissão: " + destination);
            Uri children = DocumentsContract.buildChildDocumentsUriUsingTree(tree, DocumentsContract.getDocumentId(source));
            String[] columns = {DocumentsContract.Document.COLUMN_DOCUMENT_ID, DocumentsContract.Document.COLUMN_DISPLAY_NAME};
            try (Cursor c = getContentResolver().query(children, columns, null, null, null)) {
                if (c == null) throw new IllegalArgumentException("Pasta inacessível: " + source);
                while (c.moveToNext()) {
                    String name = c.getString(1);
                    if (name == null || name.equals(".") || name.equals("..") || name.contains("/") || name.contains("\\"))
                        throw new IllegalArgumentException("Nome de arquivo inválido.");
                    copy(tree, DocumentsContract.buildDocumentUriUsingTree(tree, c.getString(0)), new File(destination, name), depth + 1);
                }
            }
        } else {
            byte[] bytes = new byte[1024 * 1024];
            try (InputStream in = getContentResolver().openInputStream(source);
                 FileOutputStream out = new FileOutputStream(destination)) {
                if (in == null) throw new IllegalArgumentException("Arquivo inacessível: " + source);
                int count;
                while ((count = in.read(bytes)) != -1) out.write(bytes, 0, count);
            }
        }
    }

    private static void deleteTree(File f) {
        if (f.isDirectory()) {
            File[] children = f.listFiles();
            if (children != null) for (File child : children) deleteTree(child);
        }
        if (f.exists() && !f.delete()) throw new IllegalStateException("Não foi possível limpar " + f);
    }
}
