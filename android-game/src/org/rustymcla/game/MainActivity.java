package org.rustymcla.game;

import android.app.Activity;
import android.app.ActivityManager;
import android.content.ClipData;
import android.content.ClipboardManager;
import android.content.Intent;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.os.Environment;
import android.provider.Settings;
import android.graphics.Color;
import android.graphics.Typeface;
import android.graphics.drawable.GradientDrawable;
import android.widget.Button;
import android.widget.ImageView;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.Spinner;
import android.widget.TextView;
import android.widget.ArrayAdapter;
import android.widget.AdapterView;
import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.io.PrintWriter;
import java.io.RandomAccessFile;
import java.util.List;
import java.util.ArrayList;

/** Launches the native engine directly from the user's shared Rusty folder. */
public final class MainActivity extends Activity {
    private static final String[] PROFILE_LABELS = {
        "Desempenho: 30 FPS", "Equilibrado: 60 FPS", "Sem limite de quadros"
    };
    private static final String[] GRAPHICS_PRESET_LABELS = {
        "Padrão — 720p e maior distância", "Desempenho — 540p e 100 m", "Personalizado"
    };
    private static final String[] RENDER_SCALE_LABELS = {
        "540p — desempenho", "720p — equilibrado", "1080p — mais nítido"
    };
    // The game's internal target follows the device's current aspect ratio.
    private static final float[] RENDER_SCALE_VALUES = {0.5f, 2.0f / 3.0f, 1.0f};
    private static final String[] BLUR_LABELS = {"Desligado", "Reduzido", "Normal"};
    private static final float[] BLUR_VALUES = {0.0f, 0.35f, 1.0f};
    private static final String[] EFFECTS_LABELS = {"Desligados", "Reduzidos (50%)", "Completos"};
    private static final float[] EFFECTS_VALUES = {0.0f, 0.5f, 1.0f};
    private static final String[] PEDESTRIAN_LABELS = {"Nenhum", "Poucos (25%)", "Metade (50%)", "Normal (100%)"};
    private static final float[] PEDESTRIAN_VALUES = {0.0f, 0.25f, 0.5f, 1.0f};
    private static final String[] DISTANCE_LABELS = {"Curta (50%)", "Média (75%)", "Longa (100%)"};
    private static final float[] DISTANCE_VALUES = {0.5f, 0.75f, 1.0f};
    private Spinner graphicsPresetSpinner, profileSpinner, renderScaleSpinner, blurSpinner, effectsSpinner, pedestrianSpinner, distanceSpinner;
    private TextView graphicsStatus;
    private boolean loadingGraphics;
    private boolean applyingGraphicsPreset;
    private TextView status;
    private Button permission;
    private Button play;
    private TextView diagnostic;
    private String diagnosticText = "";
    private Button copyDiagnostic, shareDiagnostic;
    private final java.util.concurrent.ExecutorService reports = java.util.concurrent.Executors.newSingleThreadExecutor();
    private int reportGeneration;

    @Override protected void onDestroy() {
        reportGeneration++;
        reports.shutdownNow();
        super.onDestroy();
    }

    @Override public void onCreate(Bundle state) {
        super.onCreate(state);
        installCrashRecorder();
        getWindow().setStatusBarColor(Color.rgb(11, 17, 24));
        getWindow().setNavigationBarColor(Color.rgb(11, 17, 24));
        LinearLayout panel = new LinearLayout(this);
        panel.setOrientation(LinearLayout.VERTICAL);
        panel.setPadding(dp(14), dp(10), dp(14), dp(20));
        panel.setBackgroundColor(Color.rgb(11, 17, 24));

        LinearLayout hero = card(panel);
        hero.setOrientation(LinearLayout.HORIZONTAL);
        ImageView logo = new ImageView(this);
        logo.setImageResource(getResources().getIdentifier("rusty_logo", "drawable", getPackageName()));
        LinearLayout.LayoutParams logoParams = new LinearLayout.LayoutParams(dp(76), dp(76));
        logoParams.rightMargin = dp(14);
        hero.addView(logo, logoParams);
        LinearLayout heroText = new LinearLayout(this);
        heroText.setOrientation(LinearLayout.VERTICAL);
        heroText.setGravity(android.view.Gravity.CENTER_VERTICAL);
        TextView title = new TextView(this);
        title.setText("RUSTY MCLA");
        styleText(title, 24, true, Color.rgb(246, 247, 250));
        heroText.addView(title);
        TextView subtitle = new TextView(this);
        subtitle.setText("ANDROID NATIVO  •  ARM64-V8A");
        styleText(subtitle, 11, true, Color.rgb(232, 73, 62));
        subtitle.setPadding(0, dp(4), 0, 0);
        heroText.addView(subtitle);
        hero.addView(heroText, new LinearLayout.LayoutParams(0, -2, 1));

        LinearLayout introCard = card(panel);
        addHeading(introCard, "Preparação");
        TextView explanation = new TextView(this);
        explanation.setText("Extraia as partes ZIP em Rusty/RustyMCLA na memória interna. O jogo precisa encontrar manifest.local.json e data/cache. Os dados são lidos dessa pasta sem copiar o pacote inteiro para o APK.");
        styleText(explanation, 14, false, Color.rgb(190, 199, 210));
        introCard.addView(explanation);

        LinearLayout graphicsCard = card(panel);
        TextView graphicsTitle = new TextView(this);
        graphicsTitle.setText("Gráficos e desempenho");
        styleText(graphicsTitle, 19, true, Color.rgb(246, 247, 250));
        graphicsCard.addView(graphicsTitle);
        graphicsPresetSpinner = new Spinner(this);
        graphicsPresetSpinner.setAdapter(adapter(GRAPHICS_PRESET_LABELS));
        addSpinnerRow(graphicsCard, "Predefinição gráfica", graphicsPresetSpinner);
        profileSpinner = new Spinner(this);
        profileSpinner.setAdapter(adapter(PROFILE_LABELS));
        addSpinnerRow(graphicsCard, "Limite de quadros", profileSpinner);
        TextView renderScaleTitle = new TextView(this);
        renderScaleTitle.setText("Resolução interna da cena");
        styleText(renderScaleTitle, 13, true, Color.rgb(194, 204, 216));
        renderScaleTitle.setPadding(0, dp(10), 0, 0);
        graphicsCard.addView(renderScaleTitle);
        renderScaleSpinner = new Spinner(this);
        renderScaleSpinner.setAdapter(adapter(RENDER_SCALE_LABELS));
        addSpinnerRow(graphicsCard, null, renderScaleSpinner);
        blurSpinner = new Spinner(this);
        addNamedSetting(graphicsCard, "Blur de movimento e foco", blurSpinner, BLUR_LABELS);
        effectsSpinner = new Spinner(this);
        addNamedSetting(graphicsCard, "Partículas e chuva", effectsSpinner, EFFECTS_LABELS);
        pedestrianSpinner = new Spinner(this);
        addNamedSetting(graphicsCard, "Quantidade de pedestres", pedestrianSpinner, PEDESTRIAN_LABELS);
        distanceSpinner = new Spinner(this);
        addNamedSetting(graphicsCard, "Distância de renderização", distanceSpinner, DISTANCE_LABELS);
        TextView graphicsNote = new TextView(this);
        graphicsNote.setText("540p reduz o trabalho da GPU; 720p acompanha a resolução-alvo do Xbox 360; 1080p prioriza nitidez. A interface permanece nítida.");
        styleText(graphicsNote, 12, false, Color.rgb(145, 158, 173));
        graphicsNote.setPadding(0, dp(10), 0, 0);
        graphicsCard.addView(graphicsNote);
        Button recommended = new Button(this);
        recommended.setText("Aplicar perfil de desempenho");
        styleButton(recommended);
        recommended.setBackground(tintedButton(Color.rgb(42, 81, 76)));
        recommended.setOnClickListener(v -> applyRecommendedPerformanceProfile());
        graphicsCard.addView(recommended);

        graphicsStatus = new TextView(this);
        graphicsStatus.setTextIsSelectable(true);
        styleText(graphicsStatus, 12, false, Color.rgb(161, 177, 194));
        graphicsStatus.setPadding(0, dp(10), 0, 0);
        graphicsCard.addView(graphicsStatus);
        initializeGraphicsControls();

        LinearLayout launchCard = card(panel);
        addHeading(launchCard, "Arquivos e inicialização");
        permission = new Button(this);
        permission.setText("Permitir acesso aos arquivos");
        permission.setOnClickListener(v -> requestFilesAccess());
        styleButton(permission);
        launchCard.addView(permission);
        play = new Button(this);
        play.setText("Preparar e iniciar motor");
        play.setOnClickListener(v -> prepareAndPlay());
        styleButton(play);
        play.setBackground(tintedButton(Color.rgb(190, 53, 47)));
        launchCard.addView(play);
        status = new TextView(this);
        styleText(status, 13, false, Color.rgb(190, 199, 210));
        status.setPadding(0, dp(8), 0, 0);
        launchCard.addView(status);

        LinearLayout diagnosticsCard = card(panel);
        addHeading(diagnosticsCard, "Diagnóstico");
        copyDiagnostic = new Button(this);
        copyDiagnostic.setText("Copiar diagnóstico");
        copyDiagnostic.setOnClickListener(v -> {
            ClipboardManager clipboard = (ClipboardManager) getSystemService(CLIPBOARD_SERVICE);
            clipboard.setPrimaryClip(ClipData.newPlainText("Rusty MCLA diagnóstico", diagnosticText));
            status.setText("Diagnóstico copiado. Cole o texto na conversa.");
        });
        styleButton(copyDiagnostic);
        diagnosticsCard.addView(copyDiagnostic);
        shareDiagnostic = new Button(this);
        shareDiagnostic.setText("Compartilhar diagnóstico (.txt)");
        shareDiagnostic.setOnClickListener(v -> {
            Uri uri = Uri.parse("content://" + getPackageName() + ".diagnostics/report.txt");
            Intent send = new Intent(Intent.ACTION_SEND).setType("text/plain");
            send.putExtra(Intent.EXTRA_STREAM, uri);
            send.setClipData(ClipData.newRawUri("Diagnóstico", uri));
            send.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION);
            startActivity(Intent.createChooser(send, "Enviar diagnóstico"));
        });
        styleButton(shareDiagnostic);
        diagnosticsCard.addView(shareDiagnostic);
        diagnostic = new TextView(this);
        diagnostic.setTextIsSelectable(true);
        styleText(diagnostic, 11, false, Color.rgb(158, 173, 188));
        diagnosticsCard.addView(diagnostic);
        ScrollView scroll = new ScrollView(this);
        scroll.addView(panel);
        setContentView(scroll);
    }

    private ArrayAdapter<String> adapter(String[] labels) {
        ArrayAdapter<String> result = new ArrayAdapter<String>(this, android.R.layout.simple_spinner_item, labels) {
            @Override public android.view.View getView(int position, android.view.View convertView, android.view.ViewGroup parent) {
                TextView view = (TextView) super.getView(position, convertView, parent);
                view.setTextColor(Color.rgb(240, 243, 247)); view.setTextSize(14); view.setPadding(dp(12), dp(10), dp(10), dp(10));
                return view;
            }
            @Override public android.view.View getDropDownView(int position, android.view.View convertView, android.view.ViewGroup parent) {
                TextView view = (TextView) super.getDropDownView(position, convertView, parent);
                view.setTextColor(Color.rgb(240, 243, 247)); view.setTextSize(14); view.setBackgroundColor(Color.rgb(22, 32, 43)); view.setPadding(dp(14), dp(12), dp(14), dp(12));
                return view;
            }
        };
        result.setDropDownViewResource(android.R.layout.simple_spinner_dropdown_item);
        return result;
    }

    private int dp(float value) { return (int) (value * getResources().getDisplayMetrics().density + 0.5f); }

    private LinearLayout card(LinearLayout parent) {
        LinearLayout result = new LinearLayout(this);
        result.setOrientation(LinearLayout.VERTICAL);
        result.setPadding(dp(16), dp(15), dp(16), dp(15));
        GradientDrawable bg = new GradientDrawable();
        bg.setColor(Color.rgb(21, 31, 42)); bg.setCornerRadius(dp(18));
        result.setBackground(bg); result.setElevation(dp(2));
        LinearLayout.LayoutParams params = new LinearLayout.LayoutParams(-1, -2);
        params.topMargin = dp(7); params.bottomMargin = dp(7);
        parent.addView(result, params);
        return result;
    }

    private void styleText(TextView view, float size, boolean bold, int color) {
        view.setTextSize(size); view.setTextColor(color);
        if (bold) view.setTypeface(Typeface.DEFAULT, Typeface.BOLD);
    }

    private void addHeading(LinearLayout parent, String text) {
        TextView heading = new TextView(this); heading.setText(text);
        styleText(heading, 17, true, Color.rgb(246, 247, 250));
        heading.setPadding(0, 0, 0, dp(8)); parent.addView(heading);
    }

    private void addSpinnerRow(LinearLayout parent, String title, Spinner spinner) {
        if (title != null) {
            TextView label = new TextView(this); label.setText(title);
            styleText(label, 13, true, Color.rgb(194, 204, 216));
            label.setPadding(0, dp(9), 0, 0); parent.addView(label);
        }
        spinner.setBackgroundTintList(android.content.res.ColorStateList.valueOf(Color.rgb(232, 73, 62)));
        parent.addView(spinner, new LinearLayout.LayoutParams(-1, -2));
    }

    private void addNamedSetting(LinearLayout parent, String title, Spinner spinner, String[] labels) {
        spinner.setAdapter(adapter(labels)); addSpinnerRow(parent, title, spinner);
    }

    private void styleButton(Button button) {
        button.setTextColor(Color.rgb(245, 247, 250)); button.setAllCaps(false);
        button.setTextSize(14); button.setBackground(tintedButton(Color.rgb(42, 57, 73)));
        button.setPadding(dp(14), dp(8), dp(14), dp(8));
    }

    private GradientDrawable tintedButton(int color) {
        GradientDrawable bg = new GradientDrawable(); bg.setColor(color); bg.setCornerRadius(dp(12)); return bg;
    }

    private File userDir() { return new File(getFilesDir(), "user"); }
    private File settingsFile() { return new File(userDir(), "settings.toml"); }

    private void initializeGraphicsControls() {
        loadingGraphics = true;
        File settings = settingsFile();
        if (!settings.isFile()) {
            userDir().mkdirs();
            writeSettings("schema = 1\n\n[display]\ngraphics_preset = \"performance\"\nvsync = true\nfps_cap = 30\nrender_scale = 0.5\nblur_strength = 0.0\neffects_scale = 0.0\npedestrian_density = 0.0\ndraw_distance = 0.5\n");
        }
        String selectedPreset = settingValue("display", "graphics_preset", "__missing__").trim().replace("\"", "");
        if ("__missing__".equals(selectedPreset)) {
            // Preserve values saved by older app versions, where only individual controls existed.
            selectedPreset = "custom";
            setSetting("display", "graphics_preset", "custom");
        }
        int presetIndex = "default".equalsIgnoreCase(selectedPreset) ? 0 : ("performance".equalsIgnoreCase(selectedPreset) ? 1 : 2);
        graphicsPresetSpinner.setSelection(presetIndex);
        graphicsPresetSpinner.setOnItemSelectedListener(new AdapterView.OnItemSelectedListener() {
            @Override public void onItemSelected(AdapterView<?> parent, android.view.View view, int position, long id) {
                if (loadingGraphics || applyingGraphicsPreset) return;
                if (position == 0) applyGraphicsPreset("default", 2.0f / 3.0f, 0.35f, 0.5f, 0.25f, 1.0f);
                else if (position == 1) applyGraphicsPreset("performance", 0.5f, 0.0f, 0.0f, 0.0f, 0.5f);
                else setSetting("display", "graphics_preset", "custom");
                refreshGraphicsStatus();
            }
            @Override public void onNothingSelected(AdapterView<?> parent) { }
        });
        boolean vsync = Boolean.parseBoolean(settingValue("display", "vsync", "false"));
        int cap = parseInt(settingValue("display", "fps_cap", "0"), 0);
        int profile = cap == 30 ? 0 : (cap == 60 || vsync ? 1 : 2);
        profileSpinner.setSelection(profile);
        profileSpinner.setOnItemSelectedListener(new AdapterView.OnItemSelectedListener() {
            @Override public void onItemSelected(AdapterView<?> parent, android.view.View view, int position, long id) {
                if (loadingGraphics) return;
                if (position == 0) {
                    setSetting("display", "vsync", "true"); setSetting("display", "fps_cap", "30");
                } else if (position == 1) {
                    setSetting("display", "vsync", "true"); setSetting("display", "fps_cap", "60");
                } else {
                    setSetting("display", "vsync", "false"); setSetting("display", "fps_cap", "0");
                }
                refreshGraphicsStatus();
            }
            @Override public void onNothingSelected(AdapterView<?> parent) { }
        });
        float fallbackScale = presetIndex == 1 ? 0.5f : 2.0f / 3.0f;
        float renderScale = parseFloat(settingValue("display", "render_scale", Float.toString(fallbackScale)), fallbackScale);
        int scaleIndex = 0;
        float bestScaleDelta = Float.MAX_VALUE;
        for (int i = 0; i < RENDER_SCALE_VALUES.length; i++) {
            float delta = Math.abs(RENDER_SCALE_VALUES[i] - renderScale);
            if (delta < bestScaleDelta) { bestScaleDelta = delta; scaleIndex = i; }
        }
        renderScaleSpinner.setSelection(scaleIndex);
        renderScaleSpinner.setOnItemSelectedListener(new AdapterView.OnItemSelectedListener() {
            @Override public void onItemSelected(AdapterView<?> parent, android.view.View view, int position, long id) {
                if (!loadingGraphics) {
                    setSetting("display", "render_scale", Float.toString(RENDER_SCALE_VALUES[position]));
                    markGraphicsCustom();
                    refreshGraphicsStatus();
                }
            }
            @Override public void onNothingSelected(AdapterView<?> parent) { }
        });
        bindFloatSetting(blurSpinner, BLUR_VALUES, "blur_strength", 1);
        bindFloatSetting(effectsSpinner, EFFECTS_VALUES, "effects_scale", 1);
        bindFloatSetting(pedestrianSpinner, PEDESTRIAN_VALUES, "pedestrian_density", 1);
        bindFloatSetting(distanceSpinner, DISTANCE_VALUES, "draw_distance", 1);
        loadingGraphics = false;
        refreshGraphicsStatus();
    }

    private void bindFloatSetting(Spinner spinner, float[] values, String key, int fallbackIndex) {
        float current = parseFloat(settingValue("display", key, Float.toString(values[fallbackIndex])), values[fallbackIndex]);
        int selected = fallbackIndex;
        float closest = Float.MAX_VALUE;
        for (int i = 0; i < values.length; i++) {
            float delta = Math.abs(values[i] - current);
            if (delta < closest) { closest = delta; selected = i; }
        }
        spinner.setSelection(selected);
        spinner.setOnItemSelectedListener(new AdapterView.OnItemSelectedListener() {
            @Override public void onItemSelected(AdapterView<?> parent, android.view.View view, int position, long id) {
                if (!loadingGraphics) {
                    setSetting("display", key, Float.toString(values[position]));
                    markGraphicsCustom();
                    refreshGraphicsStatus();
                }
            }
            @Override public void onNothingSelected(AdapterView<?> parent) { }
        });
    }

    private void applyRecommendedPerformanceProfile() {
        applyGraphicsPreset("performance", 0.5f, 0.0f, 0.0f, 0.0f, 0.5f);
        loadingGraphics = true;
        profileSpinner.setSelection(0);
        loadingGraphics = false;
        setSetting("display", "vsync", "true"); setSetting("display", "fps_cap", "30");
        refreshGraphicsStatus();
        status.setText("Perfil de desempenho salvo. Feche e inicie o jogo para aplicar as opções.");
    }

    private void applyGraphicsPreset(String preset, float scale, float blur, float effects, float pedestrians, float distance) {
        applyingGraphicsPreset = true;
        loadingGraphics = true;
        renderScaleSpinner.setSelection(scale <= 0.5f ? 0 : (scale < 0.9f ? 1 : 2));
        blurSpinner.setSelection(closestIndex(BLUR_VALUES, blur));
        effectsSpinner.setSelection(closestIndex(EFFECTS_VALUES, effects));
        pedestrianSpinner.setSelection(closestIndex(PEDESTRIAN_VALUES, pedestrians));
        distanceSpinner.setSelection(closestIndex(DISTANCE_VALUES, distance));
        graphicsPresetSpinner.setSelection("default".equals(preset) ? 0 : 1);
        loadingGraphics = false;
        graphicsPresetSpinner.post(() -> applyingGraphicsPreset = false);
        setSetting("display", "graphics_preset", quote(preset));
        setSetting("display", "render_scale", Float.toString(scale));
        setSetting("display", "blur_strength", Float.toString(blur));
        setSetting("display", "effects_scale", Float.toString(effects));
        setSetting("display", "pedestrian_density", Float.toString(pedestrians));
        setSetting("display", "draw_distance", Float.toString(distance));
    }

    private int closestIndex(float[] values, float target) {
        int best = 0;
        for (int i = 1; i < values.length; i++) if (Math.abs(values[i] - target) < Math.abs(values[best] - target)) best = i;
        return best;
    }

    private void markGraphicsCustom() {
        if (graphicsPresetSpinner == null) return;
        loadingGraphics = true;
        graphicsPresetSpinner.setSelection(2);
        loadingGraphics = false;
        setSetting("display", "graphics_preset", "custom");
    }

    private int parseInt(String value, int fallback) {
        try { return Integer.parseInt(value); } catch (Exception ignored) { return fallback; }
    }

    private float parseFloat(String value, float fallback) {
        try { return Float.parseFloat(value); } catch (Exception ignored) { return fallback; }
    }

    private String quote(String value) { return "\"" + value.replace("\\", "\\\\").replace("\"", "\\\"") + "\""; }

    private String settingValue(String section, String key, String fallback) {
        try {
            String current = new String(java.nio.file.Files.readAllBytes(settingsFile().toPath()), java.nio.charset.StandardCharsets.UTF_8);
            String active = "";
            for (String line : current.split("\\r?\\n")) {
                String text = line.trim();
                if (text.startsWith("[") && text.endsWith("]")) { active = text.substring(1, text.length() - 1); continue; }
                if (active.equals(section) && text.startsWith(key + " =")) return text.substring(text.indexOf('=') + 1).trim().replaceAll("^\"|\"$", "");
            }
        } catch (Exception ignored) { }
        return fallback;
    }

    private void setSetting(String section, String key, String value) {
        String current = "";
        try { if (settingsFile().isFile()) current = new String(java.nio.file.Files.readAllBytes(settingsFile().toPath()), java.nio.charset.StandardCharsets.UTF_8); }
        catch (Exception ignored) { }
        ArrayList<String> lines = new ArrayList<>();
        for (String line : current.split("\\r?\\n", -1)) lines.add(line);
        int sectionLine = -1, endLine = lines.size(), keyLine = -1;
        for (int i = 0; i < lines.size(); i++) {
            String text = lines.get(i).trim();
            if (text.startsWith("[") && text.endsWith("]")) {
                String found = text.substring(1, text.length() - 1);
                if (sectionLine >= 0 && endLine == lines.size()) endLine = i;
                if (found.equals(section)) { sectionLine = i; endLine = lines.size(); }
            } else if (sectionLine >= 0 && i > sectionLine && text.startsWith(key + " =")) keyLine = i;
        }
        if (sectionLine < 0) {
            while (!lines.isEmpty() && lines.get(lines.size() - 1).isEmpty()) lines.remove(lines.size() - 1);
            lines.add(""); lines.add("[" + section + "]"); lines.add(key + " = " + value);
        } else if (keyLine >= 0 && keyLine < endLine) {
            lines.set(keyLine, key + " = " + value);
        } else {
            lines.add(endLine, key + " = " + value);
        }
        writeSettings(joinLines(lines));
    }

    private String joinLines(List<String> lines) {
        StringBuilder result = new StringBuilder();
        for (String line : lines) result.append(line).append('\n');
        return result.toString();
    }

    private void writeSettings(String content) {
        try {
            userDir().mkdirs();
            File temp = new File(userDir(), "settings.toml.tmp");
            try (FileOutputStream out = new FileOutputStream(temp)) {
                out.write(content.getBytes(java.nio.charset.StandardCharsets.UTF_8));
                out.getFD().sync();
            }
            File target = settingsFile();
            if (target.exists() && !target.delete()) throw new java.io.IOException("não foi possível substituir settings.toml");
            if (!temp.renameTo(target)) throw new java.io.IOException("não foi possível gravar settings.toml");
        } catch (Exception error) {
            if (graphicsStatus != null) graphicsStatus.setText("Falha ao salvar as opções: " + error.getMessage());
        }
    }

    private void refreshGraphicsStatus() {
        if (graphicsStatus == null) return;
        String scale = renderScaleSpinner == null ? "720p" : RENDER_SCALE_LABELS[renderScaleSpinner.getSelectedItemPosition()];
        String blur = blurSpinner == null ? "Normal" : BLUR_LABELS[blurSpinner.getSelectedItemPosition()];
        String effects = effectsSpinner == null ? "Completos" : EFFECTS_LABELS[effectsSpinner.getSelectedItemPosition()];
        String pedestrians = pedestrianSpinner == null ? "Normal" : PEDESTRIAN_LABELS[pedestrianSpinner.getSelectedItemPosition()];
        String distance = distanceSpinner == null ? "Longa" : DISTANCE_LABELS[distanceSpinner.getSelectedItemPosition()];
        graphicsStatus.setText("Renderizador: Vulkan do sistema Android (nativo)\n"
            + "Fallback: OpenGL ES do sistema Android\n"
            + "Sem driver externo ou camada de tradução.\n"
            + "Cena: " + scale + "  •  Blur: " + blur
            + "\nEfeitos: " + effects + "  •  Pedestres: " + pedestrians + "\nDistância: " + distance
            + "\nAs opções entram em vigor ao iniciar o jogo.");
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
        if (Build.VERSION.SDK_INT < 30) return true;
        try { return (Boolean) Environment.class.getMethod("isExternalStorageManager").invoke(null); }
        catch (Exception ignored) { return false; }
    }

    private void requestFilesAccess() {
        if (Build.VERSION.SDK_INT < 30) { refresh(); return; }
        try {
            startActivity(new Intent("android.settings.MANAGE_APP_ALL_FILES_ACCESS_PERMISSION",
                Uri.parse("package:" + getPackageName())));
        } catch (Exception error) {
            startActivity(new Intent("android.settings.MANAGE_ALL_FILES_ACCESS_PERMISSION"));
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
        final int generation = ++reportGeneration;
        copyDiagnostic.setEnabled(false);
        shareDiagnostic.setEnabled(false);
        diagnostic.setText("Lendo diagnóstico…");
        reports.execute(() -> {
            String report = buildDiagnostic();
            boolean saved = false;
            try (FileOutputStream out = new FileOutputStream(new File(getFilesDir(), "diagnostic-report.tmp"))) {
                out.write(report.getBytes(java.nio.charset.StandardCharsets.UTF_8));
                File trace = new File(getFilesDir(), "diagnostic-anr.txt");
                if (trace.isFile()) {
                    out.write("\n\nANR completo (limite de 4 MiB):\n".getBytes(java.nio.charset.StandardCharsets.UTF_8));
                    try (InputStream in = new FileInputStream(trace)) {
                        byte[] bytes = new byte[8192]; int n;
                        while ((n = in.read(bytes)) != -1) out.write(bytes, 0, n);
                    }
                }
                out.flush();
                saved = new File(getFilesDir(), "diagnostic-report.tmp").renameTo(new File(getFilesDir(), "diagnostic-report.txt"));
            } catch (Exception error) { report += "\nFalha ao salvar relatório: " + error; }
            if (saved && hasFilesAccess()) {
                try {
                    File external = new File(sharedRoot(), "Rusty-MCLA-diagnostico.txt");
                    File temporary = new File(sharedRoot(), "Rusty-MCLA-diagnostico.tmp");
                    try (InputStream in = new FileInputStream(new File(getFilesDir(), "diagnostic-report.txt"));
                         FileOutputStream out = new FileOutputStream(temporary)) {
                        byte[] bytes = new byte[8192]; int n;
                        while ((n = in.read(bytes)) != -1) out.write(bytes, 0, n);
                    }
                    if (external.exists()) external.delete();
                    if (!temporary.renameTo(external)) temporary.delete();
                } catch (Exception ignored) { }
            }
            final String result = report;
            final boolean shareReady = saved;
            runOnUiThread(() -> {
                if (generation != reportGeneration || isFinishing()) return;
                diagnosticText = result;
                diagnostic.setText(result);
                copyDiagnostic.setEnabled(true);
                shareDiagnostic.setEnabled(shareReady);
            });
        });
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
                    startActivity(new Intent().setClassName(getPackageName(), "org.rustymcla.game.NativeGameActivity"));
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
        StringBuilder text = new StringBuilder("Diagnóstico Rusty MCLA 0.9\n");
        text.append("Android: ").append(Build.VERSION.RELEASE).append("; aparelho: ")
            .append(Build.MANUFACTURER).append(' ').append(Build.MODEL).append('\n');
        text.append("Acesso aos arquivos: ").append(hasFilesAccess()).append('\n');
        File root = installRoot();
        text.append("Pasta: ").append(root == null ? "não encontrada" : root.getAbsolutePath()).append('\n');
        File index = root == null ? null : new File(root, "data/cache/world_index.json");
        text.append("Índice do mundo: ").append(index != null && index.isFile() ? index.length() + " bytes" : "ausente").append('\n');
        File javaCrash = new File(getFilesDir(), "last-java-crash.txt");
        if (javaCrash.isFile()) {
            text.append("\nExceção Java:\n");
            appendFileTail(text, javaCrash);
        }
        File logs = new File(getFilesDir(), "user/logs");
        for (String prefix : new String[] {"game-", "crash-"}) {
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
        File savedTrace = new File(getFilesDir(), "diagnostic-anr.txt");
        savedTrace.delete();
        boolean traceCaptured = false;
        long started = getSharedPreferences("diagnostics", MODE_PRIVATE).getLong("last_native_launch", 0);
        // Keep the latest day of exits: force-stopping/reopening the launcher
        // must not hide the ANR that happened just before it.
        long traceWindow = System.currentTimeMillis() - 24L * 60 * 60 * 1000;
        long oldest = Math.min(started == 0 ? traceWindow : started - 1000, traceWindow);
        if (Build.VERSION.SDK_INT >= 30 && started != 0) {
            try {
                ActivityManager manager = (ActivityManager) getSystemService(ACTIVITY_SERVICE);
                @SuppressWarnings("unchecked")
                List<Object> exits = (List<Object>) ActivityManager.class
                    .getMethod("getHistoricalProcessExitReasons", String.class, int.class, int.class)
                    .invoke(manager, getPackageName(), 0, 5);
                int shown = 0;
                for (Object exit : exits) {
                    Class<?> exitClass = exit.getClass();
                    long timestamp = ((Number) exitClass.getMethod("getTimestamp").invoke(exit)).longValue();
                    int reason = ((Number) exitClass.getMethod("getReason").invoke(exit)).intValue();
                    if (timestamp >= oldest) {
                        text.append("Saída ").append(++shown).append(": motivo=").append(reason)
                            .append(", status=").append(invokeInt(exitClass, exit, "getStatus"))
                            .append(", descrição=").append(exitClass.getMethod("getDescription").invoke(exit)).append('\n');
                        // A user-confirmed ANR can be recorded by Android as a
                        // subsequent force-stop (reason 10) while retaining its
                        // ANR trace. Prefer any available trace, not only reason 6.
                        if (!traceCaptured) {
                            try (InputStream trace = (InputStream) exitClass.getMethod("getTraceInputStream").invoke(exit)) {
                                if (trace != null) {
                                    try (FileOutputStream out = new FileOutputStream(savedTrace)) {
                                        byte[] bytes = new byte[8192];
                                        int remaining = 4 * 1024 * 1024, n;
                                        while (remaining > 0 && (n = trace.read(bytes, 0, Math.min(bytes.length, remaining))) > 0) {
                                            out.write(bytes, 0, n); remaining -= n;
                                        }
                                    }
                                    traceCaptured = true;
                                    text.append("\nResumo ANR:\n");
                                    try (java.io.FileReader input = new java.io.FileReader(savedTrace)) {
                                        text.append(TraceSummary.read(input));
                                    }
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
        return text.toString();
    }

    private int invokeInt(Class<?> owner, Object target, String method) {
        try { return ((Number) owner.getMethod(method).invoke(target)).intValue(); }
        catch (Exception ignored) { return 0; }
    }

    private static void appendFileTail(StringBuilder text, File file) {
        try (RandomAccessFile input = new RandomAccessFile(file, "r")) {
            long length = input.length();
            input.seek(Math.max(0, length - 6000));
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
