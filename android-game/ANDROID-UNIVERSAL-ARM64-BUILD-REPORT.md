Relatório histórico - Rusty MCLA Android ARM64 (APK 0.9.5)
Data: 2026-10-21

Este relatório registra a build 0.9.5, não a compilação do estado atual do
checkout. Os resultados de execução mais recentes estão em TESTING-STATUS.md;
eles registram desempenho abaixo de 1 FPS e ainda não confirmam estabilidade.

OBJETIVO
Compilar o engine Rust para Android ARM64 usando APIs gráficas do sistema e sem
seleção de fabricante de GPU, driver customizado ou tradução de instruções x86.

ALTERAÇÕES
- O wgpu recebe Vulkan e GLES simultaneamente no Android. Ele escolhe um adapter
  compatível com a superfície; o backend escolhido é registrado no log do jogo.
- O build Android do wgpu habilita Vulkan e GLES. A declaração desktop continua
  separada; sdl3-sys não entra no target Android.
- Entrada de mouse relativa e botão direito ficou condicionada a desktop. Os
  caminhos compartilhados de teclado, toque e controle Android foram mantidos.
- O cpal Android usa Oboe e libc++_shared. winit continua usando
  android-activity via seu backend NativeActivity.
- O build não seleciona Adreno/Qualcomm nem importa Turnip ou outro driver.
- O empacotador Termux usa /system/framework/framework-res.apk para o link AAPT2,
  pois o android.jar do Termux serve para javac/D8, mas não traz resources.arsc.
  ANDROID_FRAMEWORK_RES pode substituir esse caminho.

BUILD
Target: aarch64-linux-android (arm64-v8a)
Comando Rust executado:
  cargo build -p mcla --lib --target aarch64-linux-android --release --features android-fps-counter

Saída:
  mcla-rs/target/aarch64-linux-android/release/libmcla_android.so
  android-game/build/Rusty-MCLA-Android-0.9.5-Universal-ARM64.apk

VERIFICAÇÃO DE BUILD
- cargo build terminou com sucesso em 15m18s no build final.
- O .so é ELF64, shared object, máquina AArch64, direcionado ao Android API 24.
- Tamanho final do .so: 155.371.200 bytes.
- A árvore Cargo Android confirmou wgpu-core com backends gles e vulkan; não
  encontrou SDL3, DX12, Metal, WinAPI, Adreno ou Turnip como dependência do alvo.
- O script AAPT2 empacotou o APK e o apksigner verify passou.
- O APK foi instalado por adb install -r e a atividade inicial foi acionada.

LIMITES CONHECIDOS
- Isso confirma compilação nativa ARM64 e seleção gráfica sem hardcoding de
  fabricante; não prova que funcione em todo aparelho Android.
- O APK define minSdk 30 e requer GLES 3.0 como mínimo declarado. Compatibilidade
  real depende dos recursos e da qualidade do driver Vulkan/GLES do sistema.
- O teste foi feito apenas em um aparelho Android 16 ARM64. Não foi feita
  sessão de jogo nesta build nem validação em GPUs Mali ou Xclipse.
- O build Rust emitiu 151 avisos; javac emitiu 3 avisos sobre source/target 8.
  Ambos terminaram sem erro.

OBSERVAÇÃO SOBRE ESTABILIDADE
O sucesso do build e da instalação não é evidência de estabilidade, taxa de FPS
ou ausência de ANR. Registrar um diagnóstico novo depois de uma sessão de jogo
será necessário para confirmar esses pontos.
