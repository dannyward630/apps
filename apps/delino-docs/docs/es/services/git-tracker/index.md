# Git Tracker

Registra mediciones numéricas para commits de Git en [git-tracker.delino.io](https://git-tracker.delino.io). Git Tracker es gratuito. Admite repositorios de GitHub públicos y privados, incluidas las solicitudes de incorporación de cambios desde forks. Úsalo para tamaños de archivos, tiempos, rendimiento, recuentos y otras mediciones escalares.

## Conectar un repositorio

1. Inicia sesión con tu cuenta de Delino y selecciona tu espacio personal o de organización.
2. Pide a un propietario del espacio que te conceda el permiso Ver, Editar o Administrar de Git Tracker a través de los grupos del espacio. Ver permite las lecturas. Editar añade las cargas y la finalización. Administrar añade la conexión, la gestión de tokens y la eliminación permanente.
3. Instala la GitHub App de Git Tracker para el repositorio.
4. Abre **Administrar repositorios**. Introduce `owner/repository` y un token de acceso de GitHub para tu cuenta de GitHub vinculada con acceso de administrador al repositorio. El servicio verifica tu autoridad en GitHub y no conserva ese token de acceso.
5. Conecta el repositorio. También necesitas autoridad de propietario/administrador del espacio de Delino. Un repositorio solo puede estar conectado a un espacio a la vez. Copia su ID de conexión desde la gestión de repositorios para las llamadas de la Action y la API. Tras la desconexión, otro espacio autorizado puede conectarlo con un nuevo ID de conexión; el historial conservado por el espacio anterior sigue siendo privado.

Las mediciones de repositorios públicos se pueden leer sin iniciar sesión. Las mediciones privadas requieren pertenecer al espacio y tener permiso de lectura de Git Tracker. El acceso al espacio se aplica a todos los repositorios conectados en ese espacio.

Desconectar o eliminar la App detiene las cargas y convierte el historial conservado en privado. Los miembros autorizados pueden seguir leyéndolo. Un cambio confirmado de público a privado convierte los datos históricos en privados. Si GitHub no puede confirmar la visibilidad actual, Git Tracker mantiene la última visibilidad confirmada hasta que la sincronización se complete correctamente.

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

Carga filas JSON, por ejemplo, una tabla de tamaños de archivos al estilo de SWC:

```json
{
  "rows": [
    { "id": "jquery", "name": "jQuery", "values": { "original": 285314, "minified": 87533, "gzip": 30321 } },
    { "id": "three", "name": "Three.js", "values": { "original": 1321000, "minified": 675000, "gzip": 170000 } }
  ]
}
```

La Action conserva la representación decimal de JSON sin una conversión a coma flotante binaria. Los clientes que llaman directamente a Connect envían los valores decimales como cadenas, por ejemplo, `"9007199254740993.0000000000000001"`. Se admiten números negativos y decimales finitos. El redondeo de visualización nunca cambia las decisiones sobre regresiones.

Las unidades admitidas son `B`, `kB`, `MB`, `GB`, `KiB`, `MiB`, `GiB`; `ns`, `μs`, `ms`, `s`; `count`, `%`, `unitless`; y `ops/s`, `B/s`, `kB/s`, `MB/s`, `GB/s`, `KiB/s`, `MiB/s`, `GiB/s`. Las unidades compatibles se convierten antes de la comparación. Los tipos incompatibles no son comparables. Por ejemplo, 1048576 B se muestra como 1 MiB; los valores exactos siguen siendo visibles en las tablas.

Las direcciones son `lower`, `higher` y `neutral`. Las columnas neutrales nunca producen regresiones. Los umbrales se aplican a todas las filas de la columna. Si hay umbrales absolutos y porcentuales, superar cualquiera de ellos constituye una regresión; igualarlo no. Sin umbral, los cambios se muestran, pero no provocan fallos por regresión. Solo se evalúan los cambios en la dirección de empeoramiento.

El cambio porcentual es `(new - baseline) / abs(baseline) * 100`. Una referencia de cero no tiene cambio porcentual. Un umbral absoluto aún puede determinar el resultado; una regla exclusivamente porcentual deja la evaluación pendiente. Las comparaciones usan la configuración almacenada de la ejecución de destino, no una configuración posterior.

## Usar la GitHub Action oficial

Fija `delinoio/apps/actions/git-tracker` a un SHA de commit completo revisado. Sustituye los marcadores de posición siguientes por tu ID de conexión de Git Tracker, la revisión de checkout y la revisión de la Action. Concede `id-token: write` para OIDC. Un token de carga del repositorio es una alternativa; guárdalo como secreto de GitHub y pasa `token`.

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

Los trabajos independientes comparten el ID de ejecución predeterminado, `repository_id/run_id/run_attempt`, y pueden cargar filas disjuntas. Usa un nuevo ID de ejecución para las reejecuciones. OIDC de Actions requiere la identidad predeterminada; los ID de ejecución personalizados son para clientes que usan tokens de repositorio. Las cargas mediante OIDC desde forks están limitadas al commit de cabecera de la PR y no pueden proporcionar referencias alternativas ni escribir en el historial de pushes del repositorio de origen.

Cada trabajo de carga evalúa solo sus propias filas cargadas. De forma predeterminada, las regresiones se notifican sin hacer fallar la CI. Establece `fail-on-regression: 'true'` para que ese trabajo falle si hay regresiones. La ausencia de comparaciones no hace fallar la CI. El trabajo de finalización solo indica que la medición ha terminado; no envía el estado de éxito, fallo o cancelación de la CI ni realiza una comprobación de aprobación agregada. Una finalización vacía es válida cuando se han omitido todos los trabajos.

Los errores de red y los fallos transitorios de la API se reintentan como máximo tres veces. De forma predeterminada, el trabajo falla tras los reintentos. Establece `warn-on-api-error: 'true'` para emitir una advertencia y aprobar en su lugar. Los fallos en los comentarios de PR solo generan advertencias; no invalidan los datos guardados ni hacen fallar la CI por sí solos. La Action registra un enlace a los detalles y escribe un resumen del trabajo.

Un único comentario de PR refleja los resultados acumulados de la última ejecución registrada en el commit más reciente de la PR. Las ejecuciones anteriores siguen disponibles en el historial web.

## Llamar a la API Connect

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

Llama a `/gittracker.v1.GitTrackerService/Complete` con el mismo `context`. Para las cargas de PR, incluye `pullRequest` y usa la rama base y el commit de cabecera de la PR. `baselineCommit` selecciona un SHA completo explícito como referencia. Los commits históricos usan la misma API de carga. No conviertas las cadenas decimales mediante `Number` de JavaScript antes de enviarlas.

## Reglas de ejecución y comparación

La primera carga fija la configuración de cada conjunto de datos para esa ejecución. Se rechaza cualquier configuración posterior diferente. Repetir una fila existente con valores numéricos idénticos es idempotente. Los valores en conflicto se rechazan. Un lote es atómico: si una fila no es válida o presenta conflictos, no se guarda nada de esa solicitud.

Los datos aparecen inmediatamente como **Midiendo**. Solo pasan a ser representativos después de la finalización. El resultado representativo de un commit es su última ejecución completada registrada, incluidas las ejecuciones con regresiones. Una ejecución anterior que termina tarde no sustituye a una ejecución completada registrada más reciente. Repetir la finalización no cambia ni la hora de finalización ni el orden.

Tras seis horas sin que se guarden nuevos datos de medición, una ejecución sin terminar pasa a estar **Interrumpida**. Las retransmisiones idénticas no reinician ese temporizador. Las ejecuciones completadas e interrumpidas rechazan cargas adicionales; las ejecuciones interrumpidas también rechazan la finalización. Inicia una nueva ejecución en su lugar. Los datos se conservan hasta que los elimines.

Las comparaciones automáticas de PR fijan el commit de base de fusión y su ejecución completada representativa en la primera carga. Una referencia ausente sigue sin estar disponible aunque se cargue más adelante. Las comparaciones automáticas de pushes recorren la ascendencia del primer padre por separado para cada fila y usan el ancestro medido más cercano que se haya completado antes del momento límite de la primera carga. Los trabajos posteriores usan ese mismo momento límite. La tabla identifica cada commit y ejecución de origen reales. Las comparaciones con un SHA de referencia explícito no recurren a ancestros alternativos por fila.

La comparación de dos commits del sitio web da preferencia a la última ejecución de rama completada registrada para cada commit seleccionado. Si un commit no tiene ninguna ejecución de rama completada, usa la última ejecución de solicitud de incorporación de cambios completada registrada para ese commit. La ejecución de destino seleccionada aporta su configuración de conjunto de datos almacenada. Las filas o columnas nuevas y los datos ausentes o incompatibles no son comparables y no provocan fallos por regresión. Los datos de referencia eliminados no se reconstruyen. Antes de la finalización, una fila ausente se considera aún no recopilada; después de la finalización, se considera faltante.

Los gráficos usan commits espaciados uniformemente en el orden del primer padre. Conectan puntos medidos reales a través de los commits omitidos y nunca arrastran ni inventan valores. Filtra por conjunto de datos, rama, columna, elementos, fechas e intervalo de commits. Usa la tabla de datos para consultar valores exactos y enlaces a ejecuciones accesibles mediante el teclado.

## Eliminación y resolución de problemas

Los administradores pueden eliminar permanentemente las mediciones de un repositorio, un conjunto de datos o una ejecución tras la confirmación. Una comparación cuya referencia se haya eliminado deja de estar disponible. Cuando Delcore finaliza la eliminación de un espacio, Git Tracker elimina las conexiones, los tokens, la configuración y el historial de ese espacio.

Si se produce un error de permisos, verifica el espacio seleccionado y tu rol actual de Git Tracker. Si hay conflictos de carga, confirma la identidad de la ejecución, la configuración del conjunto de datos y los valores completos de las filas. Para ejecuciones interrumpidas o completadas, usa un nuevo ID de ejecución. Si hay errores de red o del servidor, vuelve a intentarlo y conserva el ID de correlación de la respuesta cuando solicites ayuda. Nunca compartas tokens, credenciales ni mediciones privadas en informes públicos.

Las mediciones de ejecuciones de PR siguen pudiendo consultarse, pero no aportan puntos a los gráficos de las ramas del repositorio de origen ni referencias automáticas. Esto evita que las mediciones de forks sustituyan los datos o la configuración de la rama de referencia para un SHA de commit compartido.
