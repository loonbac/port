# Overlay generico por cuadro (OverlayHook)

Primitiva de overlay por cuadro para plugins in-process. Primer consumidor: cursor animado (port-plugins smooth-cursor).
Doctrina del dueno: APIs genericas componibles, sin nombres de caso de uso. El cursor no aparece en la API.

## Specs

S1: La semantica base se mantiene: un Some reemplaza el pintado nativo en este cuadro; None lo deja como siempre.
S2: CursorState lleva shape espejo minimo (Block/Beam/Underline/Hidden). Aunque rect sea siempre la huella de celda completa, el plugin puede emitir un overlay.rect estrecho cuando shape es Beam o Underline. No se arregla el pintado nativo de Beam/Underline.
S3: needs_animation con OR entre layout y overlay hooks. Debe responder false en cuanto la transicion termina (~120 ms), no true mientras el plugin este cargado. Las llamadas van por invoke_hook con su aislamiento y presupuesto.
S4: corner_radius_px en pixeles de punta a punta (el humano configura CORNER_RADIUS = 2.0 y espera 2 pixeles). El host lo pasa directo a corner_radii; nadie convierte nada.
S5: Invocacion incondicional, una vez por cuadro pintado, con cursor None cuando no hay cursor pintable. None significa: limpia tu estado y devuelve None.
S6: Orden del cuadro de transicion: se invoca overlay() PRIMERO y needs_animation DESPUES, para que el flag refleje el cuadro actual y el cuadro que inicia una transicion ya pida el siguiente. None pinta el nativo. La supresion se evalua solo del resultado del hook en ese mismo cuadro, sin estado pegado.
S7: Varios plugins: primer Some gana segun orden de registro; queda documentado en el hook.
S8: Plugin-side verificado por port-plugins: 21 tests en verde, clippy limpio; handoff_keeps_one_extra_frame_then_stops, idle_cursor_never_requests_frames; gracia de 32 ms y passthrough (None cuando no hay transicion).

## Evidencia

T1-T4 implementados y verificados el 2026-11-08 en la rama feature/overlay-hook.
Verde: cargo test -p port-plugin-api 54 passed (47 previos + 7 nuevos en overlay.rs);
cargo check -p port limpio; clippy -p port-plugin-api y -p port sin warnings (paint_overlay
con allow igual que paint_frame). Supresión derivada del Overlay del cuadro actual en
paint_frame; quads en paint.rs tras paint_images(false) y antes del bloque debug.

## Tasks

T1 (S1,S2,S4): tipos CellRect/CursorShape/CursorState/FrameView/OverlayQuad/Overlay + trait OverlayHook + Plugin::overlay_hook en plugin-api.
T2 (S3,S5,S7): registro: overlay() con primer-Some-gana + needs_animation con OR incluyendo overlay hooks.
T3 (S1,S5,S6): port-app: construir FrameView por cuadro, invocar siempre, suprimir inversion nativa solo si el Overlay actual lo pide, pintar quads tras glifos e imagenes y antes del chrome, opacity compuesta, radio px directo.
T4 (S3,S5,S6,S7): tests registry_contract del overlay.
T5: build + clippy + tests en nix develop; commit en la rama feature. Sin push/PR sin el humano.

## Log

L1: La sesion de port-plugins pidio animar el cursor al estilo Ghostty (duracion 0.12 s, estela 0.30 con fade, radio 2 px) como plugin in-process. El humano decidio APIs genericas componibles en vez de hooks por plugin; el contrato se cerro como OverlayHook generico con la otra sesion.
