# Git Tracker

Realiza un seguimiento de mediciones numéricas de commits de Git en [git-tracker.delino.io](https://git-tracker.delino.io). Git Tracker es gratuito. Admite repositorios públicos y privados de GitHub, incluidas las solicitudes de incorporación de cambios desde bifurcaciones. Úsalo para tamaños de archivo, tiempos, rendimiento, recuentos y otras mediciones escalares.

## Conectar un repositorio

1. Inicia sesión con tu cuenta de Delino y selecciona tu espacio personal o de organización.
2. Pide a un propietario del espacio que te conceda el permiso Ver, Editar o Administrar de Git Tracker a través de los grupos del espacio. Ver permite realizar lecturas. Editar añade cargas y finalización. Administrar añade conexión, gestión de tokens y eliminación permanente.
3. Instala la aplicación de GitHub de Git Tracker para el repositorio.
4. Abre **Administrar repositorios**. Introduce `owner/repository` y un token de acceso de GitHub para tu cuenta de GitHub vinculada con acceso de administrador al repositorio. El servicio verifica tu autoridad en GitHub y no conserva ese token de acceso.
5. Conecta el repositorio. También necesitas autoridad de propietario/administrador del espacio de Delino. Un repositorio solo puede estar conectado a un espacio a la vez. Copia su ID de conexión desde la gestión de repositorios para las llamadas de la Action y la API. Tras la desconexión, otro espacio autorizado puede conectarlo con un nuevo ID de conexión; el historial conservado del espacio anterior sigue siendo privado.

Las mediciones de repositorios públicos se pueden leer sin iniciar sesión. Las mediciones privadas requieren pertenecer al espacio y tener permiso de lectura de Git Tracker. El acceso al espacio se aplica a todos los repositorios conectados en ese espacio.

Desconectar o eliminar la aplicación detiene las cargas y hace que el historial conservado sea privado. Los miembros autorizados pueden seguir leyéndolo. Un cambio confirmado de público a privado hace que los datos históricos sean privados. Si GitHub no puede confirmar la visibilidad actual, Git Tracker mantiene la última visibilidad confirmada hasta que la sincronización se complete correctamente.

## Definir tu conjunto de datos

Mantén una configuración YAML en tu repositorio. Los ID son estables; los nombres pueden cambiar sin perder el historial. Todas las columnas de una fila deben cargarse juntas.

```yaml
id: file-size
name: JavaScript file sizes
columns:
  - id: original
    name: Original size
    unit: B
    direction: lower
    absoluteThreshold: 1024
  - id: minified
    name: Minified size
    unit: B
    direction: lower
    percentageThreshold: 1
  - id: gzip
    name: Gzipped size
    unit: B
    direction: lower
    absoluteThreshold: 128
    percentageThreshold: 2
```

Carga filas JSON, por ejemplo, una tabla de tamaños de archivo al estilo de SWC:

```json
{
  "rows": [
    { "id": "jquery", "name": "jQuery", "values": { "original": 285314, "minified": 87533, "gzip": 30321 } },
    { "id": "three", "name": "Three.js", "values": { "original": 1321000, "minified": 675000, "gzip": 170000 } }
  ]
}
```

La Action conserva la representación decimal de JSON sin convertirla a coma flotante binaria. Los clientes que llaman directamente a Connect envían los valores decimales como cadenas, como `"9007199254740993.0000000000000001"`. Se admiten números negativos y decimales finitos. El redondeo de visualización nunca altera las decisiones sobre regresiones.

Las unidades admitidas son `B`, `kB`, `MB`, `GB`, `KiB`, `MiB`, `GiB`; `ns`, `μs`, `ms`, `s`; `count`, `%`, `unitless`; y `ops/s`, `B/s`, `kB/s`, `MB/s`, `GB/s`, `KiB/s`, `MiB/s`, `GiB/s`. Las unidades compatibles se convierten antes de la comparación. Los tipos incompatibles no son comparables. Por ejemplo, 1048576 B se muestra como 1 MiB; los valores exactos siguen siendo visibles en las tablas.

Las direcciones son `lower`, `higher` y `neutral`. Las columnas neutrales nunca producen regresiones. Los umbrales se aplican a todas las filas de la columna. Cuando hay umbrales absolutos y porcentuales, superar cualquiera de ellos constituye una regresión; igualarlo no. Sin umbral, los cambios se muestran, pero no provocan fallos por regresión. Solo se evalúan los cambios en la dirección de empeoramiento.

El cambio porcentual es `(new - baseline) / abs(baseline) * 100`. Un valor de referencia de cero no tiene cambio porcentual. Un umbral absoluto aún puede determinar el resultado; una regla solo porcentual aplaza la evaluación. Las comparaciones usan la configuración almacenada de la ejecución de destino, no una configuración posterior.

## Usar la GitHub Action oficial

Fija `delinoio/apps/actions/git-tracker` a un SHA de commit completo y revisado. Sustituye los marcadores de posición siguientes por tu ID de conexión de Git Tracker, la revisión de checkout y la revisión de la Action. Concede `id-token: write` para OIDC. Un token de carga del repositorio es una alternativa; guárdalo como secreto de GitHub y pasa `token`.

```yaml
permissions:
  contents: read
  id-token: write
jobs:
  measure:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@<reviewed-checkout-commit-sha>
      - run: ./measure.sh
      - uses: delinoio/apps/actions/git-tracker@<reviewed-action-commit-sha>
        with:
          repository-id: '019b0000-0000-7000-8000-000000000001'
          configuration: measurements.yaml
          measurements: measurements.json
          fail-on-regression: 'false'
  mark-as-done:
    needs: measure
    if: always()
    runs-on: ubuntu-latest
    steps:
      - uses: delinoio/apps/actions/git-tracker@<reviewed-action-commit-sha>
        with:
          operation: complete
          repository-id: '019b0000-0000-7000-8000-000000000001'
```

Los trabajos independientes comparten el ID de ejecución predeterminado, `repository_id/run_id/run_attempt`, y pueden cargar filas disjuntas. Usa un ID de ejecución nuevo para las repeticiones. OIDC de Actions requiere la identidad predeterminada; los ID de ejecución personalizados son para los clientes que usan tokens de repositorio. Para un trabajo de solicitud de incorporación de cambios cuya credencial OIDC pertenezca al repositorio conectado, establece la entrada `baseline` de la Action para usar un SHA de commit completo explícito. Las cargas OIDC desde bifurcaciones están limitadas al commit de cabecera de la PR y no pueden proporcionar sustituciones de la referencia ni escribir en el historial de envíos del repositorio de origen.

Cada trabajo de carga evalúa solo sus propias filas cargadas. De forma predeterminada, las regresiones se notifican sin hacer fallar la CI. Establece `fail-on-regression: 'true'` para que ese trabajo falle si hay regresiones. Las comparaciones ausentes no hacen fallar la CI. El trabajo de finalización solo indica que la medición ha terminado; no envía el resultado de éxito, fallo o cancelación de la CI ni realiza una validación agregada. Una finalización vacía es válida cuando se han omitido todos los trabajos.

Los errores de red y los fallos transitorios de la API se reintentan como máximo tres veces. De forma predeterminada, el trabajo falla después de los reintentos. Establece `warn-on-api-error: 'true'` para emitir una advertencia y finalizar correctamente en su lugar. Los fallos en los comentarios de las PR solo generan advertencias; no invalidan los datos guardados ni hacen fallar la CI por sí solos. La Action registra un enlace a los detalles y escribe un resumen del trabajo.

Un único comentario de la PR refleja los resultados acumulados de la última ejecución registrada en el commit más reciente de la PR. Las ejecuciones anteriores siguen disponibles en el historial web.

## Llamar a la API de Connect

El origen de la API es `https://git-tracker.api.delino.io`. Las solicitudes JSON de Connect usan `Content-Type: application/json` y `Connect-Protocol-Version: 1`. Los administradores pueden emitir tokens de repositorio con una fecha de caducidad futura obligatoria y revocarlos desde la gestión de repositorios.

```sh
curl --fail-with-body \
  'https://git-tracker.api.delino.io/gittracker.v1.GitTrackerService/Upload' \
  -H "Authorization: Bearer ${GIT_TRACKER_TOKEN}" \
  -H 'Content-Type: application/json' \
  -H 'Connect-Protocol-Version: 1' \
  --data-binary @upload.json
```

`upload.json` contiene `context`, `dataset` y `rows`:

```json
{
  "context": {
    "repositoryId": "019b0000-0000-7000-8000-000000000001",
    "executionId": "my-historical-run",
    "commit": "0123456789012345678901234567890123456789",
    "branch": "main"
  },
  "dataset": {
    "id": "size",
    "name": "Size",
    "columns": [{ "id": "bytes", "name": "Bytes", "unit": "B", "direction": "DIRECTION_LOWER", "absoluteThreshold": "1024" }]
  },
  "rows": [{ "id": "bundle", "name": "Bundle", "values": { "bytes": "1048576" } }]
}
```

Llama a `/gittracker.v1.GitTrackerService/Complete` con el mismo `context`. Para las cargas de PR, incluye `pullRequest` y usa la rama base y el commit de cabecera de la PR. `baselineCommit` selecciona un SHA completo explícito de referencia. Los commits históricos usan la misma API de carga. No conviertas las cadenas decimales mediante `Number` de JavaScript antes de enviarlas.

No incluyas un carácter NUL (`\u0000`) en el texto enviado con una carga o finalización. Git Tracker lo rechaza con un error de cliente que identifica el campo afectado. El resto del texto Unicode y los caracteres de control admitidos siguen siendo válidos.

## Reglas de ejecución y comparación

La primera carga fija la configuración de cada conjunto de datos para esa ejecución. Cualquier configuración posterior diferente se rechaza. Repetir una fila existente con valores numéricos idénticos es idempotente. Los valores en conflicto se rechazan. Un lote es atómico: una fila no válida o en conflicto impide que se guarde cualquier dato de esa solicitud.

Los datos aparecen inmediatamente con el estado **Midiendo**. Solo pasan a ser representativos tras la finalización. El resultado representativo de un commit es su última ejecución registrada y completada, incluidas las ejecuciones con regresiones. Una ejecución anterior que termine tarde no sustituye a una ejecución completada registrada más recientemente. Repetir la finalización no cambia ni la hora de finalización ni el orden.

Tras seis horas sin nuevos datos de medición guardados, una ejecución sin finalizar pasa al estado **Interrumpida**. Las retransmisiones idénticas no reinician ese temporizador. Las ejecuciones completadas e interrumpidas rechazan más cargas; las ejecuciones interrumpidas también rechazan la finalización. Inicia una nueva ejecución en su lugar. Los datos se conservan hasta que los elimines.

Las comparaciones automáticas de PR fijan el commit de base de fusión y su ejecución completada representativa en la primera carga. Una referencia ausente sigue sin estar disponible aunque se cargue más tarde. Las comparaciones automáticas de envíos comprueban hasta los 1.000 ancestros más cercanos siguiendo el primer padre por separado para cada fila y usan el ancestro medido más cercano que se haya completado antes del límite temporal de la primera carga. Si no hay ninguna medición de fila apta dentro de ese intervalo, la comparación no está disponible. Los trabajos posteriores usan ese mismo límite temporal. La tabla identifica cada commit y ejecución de origen reales. Puedes establecer un SHA de referencia explícito para comparar con un commit anterior; las comparaciones explícitas no recurren a ancestros alternativos por fila.

La comparación de dos commits del sitio web da preferencia a la última ejecución de rama registrada y completada para cada commit seleccionado. Si un commit no tiene ninguna ejecución de rama completada, usa la última ejecución de solicitud de incorporación de cambios registrada y completada para ese commit. La ejecución de destino seleccionada proporciona su configuración de conjunto de datos almacenada. Las filas o columnas nuevas y los datos ausentes o incompatibles no son comparables y no provocan fallos por regresión. Los datos de referencia eliminados no se reconstruyen. Antes de la finalización, una fila ausente se considera no recopilada; después de la finalización, se considera faltante.

Los gráficos usan commits espaciados uniformemente en orden de primer padre. Conectan puntos medidos reales a través de los commits omitidos y nunca arrastran ni inventan valores. Filtra por conjunto de datos, rama, columna, elementos, fechas e intervalo de commits. Usa la tabla de datos para consultar valores exactos y enlaces a ejecuciones accesibles mediante el teclado.

## Eliminación y resolución de problemas

Los administradores pueden eliminar permanentemente las mediciones de un repositorio, un conjunto de datos o una ejecución tras confirmarlo. Al eliminar una ejecución desconocida o un conjunto de datos sin mediciones actuales, se informa de que no se ha encontrado el destino. Eliminar un conjunto de datos solo afecta a las ejecuciones que lo contenían; otras ejecuciones aún pueden cargar ese conjunto de datos. Una comparación cuyas mediciones de referencia se hayan eliminado deja de estar disponible, mientras que una referencia que nunca contenía el conjunto de datos lo trata como datos nuevos. Cuando Delcore finaliza la eliminación del espacio, Git Tracker elimina las conexiones, los tokens, la configuración y el historial del espacio.

Si se produce un error de permisos, verifica el espacio seleccionado y tu rol actual de Git Tracker. Si hay conflictos de carga, confirma la identidad de la ejecución, la configuración del conjunto de datos y los valores completos de las filas. Para ejecuciones interrumpidas o completadas, usa un ID de ejecución nuevo. Para errores de red o del servidor, vuelve a intentarlo y conserva el ID de correlación de la respuesta cuando solicites ayuda. Nunca compartas tokens, credenciales ni mediciones privadas en informes públicos.

Las mediciones de ejecuciones de PR siguen pudiéndose inspeccionar, pero no proporcionan puntos para los gráficos de las ramas del repositorio de origen ni referencias automáticas. Esto impide que las mediciones de bifurcaciones sustituyan los datos o la configuración de la rama de referencia para un SHA de commit compartido.
