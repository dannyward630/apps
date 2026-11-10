# Git Tracker

Verfolge numerische Messwerte für Git-Commits unter [git-tracker.delino.io](https://git-tracker.delino.io). Git Tracker ist kostenlos. Es unterstützt öffentliche und private GitHub-Repositories, einschließlich Pull Requests aus Forks. Nutze es für Dateigrößen, Laufzeiten, Durchsatz, Anzahlen und andere skalare Messwerte.

## Ein Repository verbinden

1. Melde dich mit deinem Delino-Konto an und wähle deinen persönlichen oder Organisations-Space aus.
2. Bitte einen Space-Eigentümer, dir über die Gruppen des Spaces die Git-Tracker-Berechtigung View, Edit oder Manage zu erteilen. View erlaubt Lesezugriffe. Edit ermöglicht zusätzlich Uploads und den Abschluss von Ausführungen. Manage ermöglicht zusätzlich das Verbinden, die Token-Verwaltung und das endgültige Löschen.
3. Installiere die Git Tracker GitHub App für das Repository.
4. Öffne **Repositories verwalten**. Gib `owner/repository` und ein GitHub-Zugriffstoken für dein verknüpftes GitHub-Konto mit Administratorzugriff auf das Repository ein. Der Dienst überprüft deine GitHub-Berechtigung und speichert dieses Zugriffstoken nicht.
5. Verbinde das Repository. Du benötigst außerdem Owner-/Admin-Berechtigungen für den Delino-Space. Ein Repository kann jeweils nur mit einem Space verbunden sein. Kopiere seine Verbindungs-ID aus der Repository-Verwaltung für Action- und API-Aufrufe. Nach dem Trennen der Verbindung kann ein anderer berechtigter Space es unter einer neuen Verbindungs-ID verbinden; der im alten Space aufbewahrte Verlauf bleibt privat.

Messwerte öffentlicher Repositories sind ohne Anmeldung lesbar. Private Messwerte erfordern die Mitgliedschaft im Space und die Git-Tracker-Leseberechtigung. Der Space-Zugriff gilt für alle verbundenen Repositories in diesem Space.

Das Trennen der Verbindung oder das Entfernen der App stoppt Uploads und macht den aufbewahrten Verlauf privat. Berechtigte Mitglieder können ihn weiterhin lesen. Ein bestätigter Wechsel von öffentlich zu privat macht historische Daten privat. Wenn GitHub die aktuelle Sichtbarkeit nicht bestätigen kann, behält Git Tracker die zuletzt bestätigte Sichtbarkeit bei, bis die Synchronisierung erfolgreich ist.

## Deinen Datensatz definieren

Lege eine YAML-Konfiguration in deinem Repository ab. IDs sind stabil; Namen können geändert werden, ohne dass der Verlauf verloren geht. Alle Spalten einer Zeile müssen zusammen hochgeladen werden.

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

Lade JSON-Zeilen hoch, zum Beispiel eine Dateigrößentabelle im SWC-Stil:

```json
{
  "rows": [
    { "id": "jquery", "name": "jQuery", "values": { "original": 285314, "minified": 87533, "gzip": 30321 } },
    { "id": "three", "name": "Three.js", "values": { "original": 1321000, "minified": 675000, "gzip": 170000 } }
  ]
}
```

Die Action bewahrt die Dezimalschreibweise von JSON ohne Umwandlung in binäre Gleitkommazahlen. Direkte Connect-Aufrufer senden Dezimalwerte als Zeichenfolgen, etwa `"9007199254740993.0000000000000001"`. Negative Zahlen und endliche Dezimalzahlen werden unterstützt. Die Rundung für die Anzeige verändert niemals Regressionsentscheidungen.

Unterstützte Einheiten sind `B`, `kB`, `MB`, `GB`, `KiB`, `MiB`, `GiB`; `ns`, `μs`, `ms`, `s`; `count`, `%`, `unitless`; sowie `ops/s`, `B/s`, `kB/s`, `MB/s`, `GB/s`, `KiB/s`, `MiB/s`, `GiB/s`. Kompatible Einheiten werden vor dem Vergleich umgerechnet. Inkompatible Arten sind nicht vergleichbar. Beispielsweise werden 1048576 B als 1 MiB angezeigt; exakte Werte bleiben in Tabellen sichtbar.

Die Richtungen sind `lower`, `higher` und `neutral`. Neutrale Spalten erzeugen niemals Regressionen. Schwellenwerte gelten für alle Zeilen in der Spalte. Sind sowohl absolute als auch prozentuale Schwellenwerte vorhanden, gilt das Überschreiten eines der beiden als Regression; Gleichheit gilt nicht als Regression. Ohne Schwellenwert werden Änderungen angezeigt, verursachen aber keine regressionsbedingten Fehlschläge. Nur Änderungen in die verschlechternde Richtung werden bewertet.

Die prozentuale Änderung ist `(new - baseline) / abs(baseline) * 100`. Bei einem Basiswert von null gibt es keine prozentuale Änderung. Ein absoluter Schwellenwert kann dennoch eine Entscheidung ermöglichen; eine rein prozentuale Regel setzt die Bewertung aus. Vergleiche verwenden die gespeicherte Konfiguration der Zielausführung, nicht eine spätere Konfiguration.

## Die offizielle GitHub Action verwenden

Fixiere `delinoio/apps/actions/git-tracker` auf einen überprüften vollständigen Commit-SHA. Ersetze die folgenden Platzhalter durch deine Git-Tracker-Verbindungs-ID, die Checkout-Revision und die Action-Revision. Erteile `id-token: write` für OIDC. Ein Repository-Upload-Token ist eine Alternative; speichere es als GitHub-Secret und übergib `token`.

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

Unabhängige Jobs teilen sich die Standard-Ausführungs-ID `repository_id/run_id/run_attempt` und können überschneidungsfreie Zeilen hochladen. Verwende für erneute Ausführungen eine neue Ausführungs-ID. Actions-OIDC erfordert die Standardidentität; benutzerdefinierte Ausführungs-IDs sind für Aufrufer mit Repository-Token vorgesehen. OIDC-Uploads aus Forks sind auf den PR-Head beschränkt und können weder die Vergleichsbasis überschreiben noch den Upstream-Push-Verlauf beschreiben.

Jeder Upload-Job bewertet nur seine eigenen hochgeladenen Zeilen. Regressionen werden standardmäßig gemeldet, ohne die CI fehlschlagen zu lassen. Setze `fail-on-regression: 'true'`, damit dieser Job bei Regressionen fehlschlägt. Fehlende Vergleiche lassen die CI nicht fehlschlagen. Der Abschluss-Job gibt nur an, dass die Messung beendet ist; er übermittelt weder CI-Erfolg, -Fehlschlag oder -Abbruch noch führt er eine zusammenfassende Freigabeprüfung durch. Ein leerer Abschluss ist gültig, wenn alle Jobs übersprungen wurden.

Bei Netzwerkfehlern und vorübergehenden API-Fehlern erfolgen höchstens drei Wiederholungsversuche. Standardmäßig schlägt der Job nach den Wiederholungsversuchen fehl. Setze `warn-on-api-error: 'true'`, um stattdessen eine Warnung auszugeben und den Job erfolgreich abzuschließen. Fehler bei PR-Kommentaren führen nur zu Warnungen; sie machen gespeicherte Daten nicht ungültig und lassen die CI nicht eigenständig fehlschlagen. Die Action protokolliert einen Detail-Link und schreibt eine Job-Zusammenfassung.

Ein einzelner PR-Kommentar zeigt die kumulierten Ergebnisse der zuletzt registrierten Ausführung für den neuesten Commit des PRs. Ältere Ausführungen bleiben im Webverlauf verfügbar.

## Die Connect API aufrufen

Der API-Ursprung ist `https://git-tracker.api.delino.io`. Connect-JSON-Anfragen verwenden `Content-Type: application/json` und `Connect-Protocol-Version: 1`. Verwalter können Repository-Tokens mit einem verpflichtenden Ablaufdatum in der Zukunft ausstellen und sie in der Repository-Verwaltung widerrufen.

```sh
curl --fail-with-body \
  'https://git-tracker.api.delino.io/gittracker.v1.GitTrackerService/Upload' \
  -H "Authorization: Bearer ${GIT_TRACKER_TOKEN}" \
  -H 'Content-Type: application/json' \
  -H 'Connect-Protocol-Version: 1' \
  --data-binary @upload.json
```

`upload.json` enthält `context`, `dataset` und `rows`:

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

Rufe `/gittracker.v1.GitTrackerService/Complete` mit demselben `context` auf. Füge für PR-Uploads `pullRequest` hinzu und verwende den Basis-Branch und den Head-Commit des PRs. `baselineCommit` wählt einen expliziten vollständigen Basis-SHA aus. Historische Commits verwenden dieselbe Upload-API. Wandle Dezimalzeichenfolgen vor dem Senden nicht über JavaScript-`Number` um.

## Regeln für Ausführungen und Vergleiche

Der erste Upload schreibt die Konfiguration jedes Datensatzes für diese Ausführung fest. Eine abweichende spätere Konfiguration wird abgelehnt. Das erneute Übermitteln einer vorhandenen Zeile mit identischen numerischen Werten ist idempotent. Widersprüchliche Werte werden abgelehnt. Ein Batch ist atomar: Bei einer ungültigen oder widersprüchlichen Zeile wird nichts aus dieser Anfrage gespeichert.

Daten erscheinen sofort als **Messung läuft**. Sie werden erst nach dem Abschluss repräsentativ. Das repräsentative Ergebnis für einen Commit ist seine zuletzt registrierte abgeschlossene Ausführung, einschließlich Ausführungen mit Regressionen. Eine ältere Ausführung, die verspätet endet, ersetzt keine neuer registrierte abgeschlossene Ausführung. Ein wiederholter Abschluss verändert weder den Abschlusszeitpunkt noch die Reihenfolge.

Nach sechs Stunden ohne neu gespeicherte Messdaten wird eine nicht abgeschlossene Ausführung als **Unterbrochen** markiert. Identische erneute Übertragungen setzen diesen Timer nicht zurück. Abgeschlossene und unterbrochene Ausführungen lehnen weitere Uploads ab; unterbrochene Ausführungen lehnen auch den Abschluss ab. Starte stattdessen eine neue Ausführung. Die Daten bleiben erhalten, bis du sie löschst.

Automatische PR-Vergleiche schreiben den Merge-Base-Commit und seine repräsentative abgeschlossene Ausführung beim ersten Upload fest. Eine fehlende Vergleichsbasis bleibt auch dann nicht verfügbar, wenn sie später hochgeladen wird. Automatische Push-Vergleiche durchlaufen die First-Parent-Abstammung für jede Zeile separat und verwenden den nächstgelegenen gemessenen Vorfahren, dessen Ausführung bis zum Stichtag des ersten Uploads abgeschlossen war. Spätere Jobs verwenden denselben Stichtag. Die Tabelle weist jeden tatsächlich verwendeten Quell-Commit und die zugehörige Ausführung aus. Vergleiche mit explizitem Basis-SHA verwenden keinen zeilenweisen Rückgriff auf Vorfahren.

Der Zwei-Commit-Vergleich der Website bevorzugt für jeden ausgewählten Commit die zuletzt registrierte abgeschlossene Branch-Ausführung. Hat ein Commit keine abgeschlossene Branch-Ausführung, wird die zuletzt registrierte abgeschlossene Pull-Request-Ausführung für diesen Commit verwendet. Die ausgewählte Zielausführung liefert ihre gespeicherte Datensatzkonfiguration. Neue Zeilen/Spalten sowie fehlende oder inkompatible Daten sind nicht vergleichbar und verursachen keinen regressionsbedingten Fehlschlag. Gelöschte Basisdaten werden nicht rekonstruiert. Vor dem Abschluss gilt eine nicht vorhandene Zeile als noch nicht erfasst; nach dem Abschluss gilt sie als fehlend.

Diagramme verwenden gleichmäßig verteilte Commits in First-Parent-Reihenfolge. Sie verbinden tatsächlich gemessene Punkte über übersprungene Commits hinweg und schreiben niemals Werte fort oder erfinden welche. Filtere nach Datensatz, Branch, Spalte, Elementen, Datumsangaben und Commit-Bereich. Verwende die Datentabelle für exakte Werte und per Tastatur zugängliche Ausführungslinks.

## Löschen und Fehlerbehebung

Verwalter können die Messwerte eines Repositories, einen Datensatz oder eine einzelne Ausführung nach Bestätigung endgültig löschen. Ein Vergleich, dessen Vergleichsbasis gelöscht wurde, ist nicht mehr verfügbar. Wenn Delcore die Löschung eines Spaces abschließt, löscht Git Tracker die Verbindungen, Tokens, Konfiguration und den Verlauf des Spaces.

Überprüfe bei einem Berechtigungsfehler den ausgewählten Space und deine aktuelle Git-Tracker-Rolle. Überprüfe bei Upload-Konflikten die Ausführungsidentität, die Datensatzkonfiguration und die vollständigen Zeilenwerte. Verwende für unterbrochene oder abgeschlossene Ausführungen eine neue Ausführungs-ID. Versuche es bei Netzwerk-/Serverfehlern erneut und bewahre die Korrelations-ID der Antwort auf, wenn du um Hilfe bittest. Teile niemals Tokens, Zugangsdaten oder private Messwerte in öffentlichen Berichten.

Messwerte aus PR-Ausführungen bleiben einsehbar, liefern aber weder Punkte für Upstream-Branch-Diagramme noch automatische Vergleichsbasen. Dadurch wird verhindert, dass Messwerte aus Forks die Daten oder Konfiguration des Basis-Branches für einen gemeinsamen Commit-SHA ersetzen.
