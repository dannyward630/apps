# Git Tracker

Acompanhe medições numéricas de commits Git em [git-tracker.delino.io](https://git-tracker.delino.io). O Git Tracker é gratuito. Ele oferece suporte a repositórios públicos e privados do GitHub, incluindo pull requests de forks. Use-o para tamanhos de arquivos, tempos, taxas de transferência, contagens e outras medições escalares.

## Conectar um repositório

1. Entre com sua conta Delino e selecione seu espaço pessoal ou da organização.
2. Peça a um proprietário do espaço que conceda a permissão View, Edit ou Manage do Git Tracker por meio dos grupos do espaço. View permite leituras. Edit acrescenta envios e conclusão. Manage acrescenta conexão, gerenciamento de tokens e exclusão permanente.
3. Instale o Git Tracker GitHub App para o repositório.
4. Abra **Gerenciar repositórios**. Insira `owner/repository` e um token de acesso do GitHub para sua conta vinculada do GitHub com acesso de administrador ao repositório. O serviço verifica sua autoridade no GitHub e não retém esse token de acesso.
5. Conecte o repositório. Você também precisa de autoridade de Owner/Admin no espaço Delino. Um repositório só pode estar conectado a um espaço por vez. Copie o ID de conexão no gerenciamento de repositórios para chamadas da Action e da API. Após a desconexão, outro espaço autorizado pode conectá-lo com um novo ID de conexão; o histórico retido do espaço antigo permanece privado.

As medições de repositórios públicos podem ser lidas sem entrar na conta. As medições privadas exigem participação no espaço e permissão de leitura do Git Tracker. O acesso ao espaço se aplica a todos os repositórios conectados nesse espaço.

Desconectar ou remover o App interrompe os envios e torna privado o histórico retido. Membros autorizados ainda podem lê-lo. Uma mudança confirmada de público para privado torna privados os dados históricos. Se o GitHub não puder confirmar a visibilidade atual, o Git Tracker mantém a última visibilidade confirmada até que a sincronização seja bem-sucedida.

## Definir seu conjunto de dados

Mantenha uma configuração YAML no seu repositório. Os IDs são estáveis; os nomes podem mudar sem perda de histórico. Todas as colunas de uma linha devem ser enviadas juntas.

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

Envie linhas JSON, por exemplo, uma tabela de tamanhos de arquivos no estilo SWC:

```json
{
  "rows": [
    { "id": "jquery", "name": "jQuery", "values": { "original": 285314, "minified": 87533, "gzip": 30321 } },
    { "id": "three", "name": "Three.js", "values": { "original": 1321000, "minified": 675000, "gzip": 170000 } }
  ]
}
```

A Action preserva a representação decimal do JSON sem conversão para ponto flutuante binário. Clientes que chamam o Connect diretamente enviam valores decimais como strings, como `"9007199254740993.0000000000000001"`. Há suporte a números negativos e decimais finitos. O arredondamento na exibição nunca altera as decisões sobre regressões.

As unidades compatíveis são `B`, `kB`, `MB`, `GB`, `KiB`, `MiB`, `GiB`; `ns`, `μs`, `ms`, `s`; `count`, `%`, `unitless`; e `ops/s`, `B/s`, `kB/s`, `MB/s`, `GB/s`, `KiB/s`, `MiB/s`, `GiB/s`. Unidades compatíveis são convertidas antes da comparação. Tipos incompatíveis não são comparáveis. Por exemplo, 1048576 B é exibido como 1 MiB; os valores exatos permanecem visíveis nas tabelas.

As direções são `lower`, `higher` e `neutral`. Colunas neutras nunca produzem regressões. Os limites se aplicam a todas as linhas da coluna. Quando há limites absolutos e percentuais, exceder qualquer um deles é uma regressão; a igualdade não é. Sem limite, as mudanças são exibidas, mas não causam falhas por regressão. Somente mudanças na direção de piora são avaliadas.

A variação percentual é `(new - baseline) / abs(baseline) * 100`. Uma linha de base igual a zero não tem variação percentual. Um limite absoluto ainda pode determinar o resultado; uma regra apenas percentual não emite julgamento. As comparações usam a configuração armazenada da execução de destino, não uma configuração posterior.

## Usar a GitHub Action oficial

Fixe `delinoio/apps/actions/git-tracker` em um SHA completo de commit revisado. Substitua os placeholders abaixo pelo ID de conexão do Git Tracker, pela revisão de checkout e pela revisão da Action. Conceda `id-token: write` para OIDC. Um token de envio do repositório é uma alternativa; armazene-o como um segredo do GitHub e passe `token`.

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

Jobs independentes compartilham o ID de execução padrão, `repository_id/run_id/run_attempt`, e podem enviar conjuntos distintos de linhas. Use um novo ID de execução para reexecuções. O OIDC do Actions exige a identidade padrão; IDs de execução personalizados são para clientes que usam tokens de repositório. Envios via OIDC de forks são limitados ao head do PR e não podem fornecer substituições da linha de base nem gravar no histórico de pushes do repositório de origem.

Cada job de envio avalia apenas as próprias linhas enviadas. Por padrão, as regressões são relatadas sem fazer a CI falhar. Defina `fail-on-regression: 'true'` para fazer esse job falhar em caso de regressões. Comparações ausentes não fazem a CI falhar. O job de conclusão apenas informa que a medição terminou; ele não envia sucesso/falha/cancelamento da CI nem realiza verificação agregada para bloqueio. Uma conclusão vazia é válida quando todos os jobs foram ignorados.

Erros de rede e falhas transitórias da API são repetidos no máximo três vezes. Por padrão, o job falha após as novas tentativas. Defina `warn-on-api-error: 'true'` para emitir um aviso e passar. Falhas em comentários de PR apenas emitem avisos; elas não invalidam os dados salvos nem fazem a CI falhar por si só. A Action registra um link de detalhes e escreve um resumo do job.

Um comentário no PR reflete os resultados acumulados da execução registrada mais recentemente no commit mais recente do PR. Execuções anteriores permanecem disponíveis no histórico da web.

## Chamar a API Connect

A origem da API é `https://git-tracker.api.delino.io`. As solicitações JSON do Connect usam `Content-Type: application/json` e `Connect-Protocol-Version: 1`. Gerentes podem emitir tokens de repositório com uma data de expiração futura obrigatória e revogá-los no gerenciamento de repositórios.

```sh
curl --fail-with-body \
  'https://git-tracker.api.delino.io/gittracker.v1.GitTrackerService/Upload' \
  -H "Authorization: Bearer ${GIT_TRACKER_TOKEN}" \
  -H 'Content-Type: application/json' \
  -H 'Connect-Protocol-Version: 1' \
  --data-binary @upload.json
```

`upload.json` contém `context`, `dataset` e `rows`:

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

Chame `/gittracker.v1.GitTrackerService/Complete` com o mesmo `context`. Para envios de PR, inclua `pullRequest` e use a branch base e o commit head do PR. `baselineCommit` seleciona um SHA completo explícito da linha de base. Commits históricos usam a mesma API de envio. Não converta strings decimais por meio de `Number` do JavaScript antes de enviá-las.

## Regras de execução e comparação

O primeiro envio fixa a configuração de cada conjunto de dados para essa execução. Uma configuração posterior diferente é rejeitada. Repetir uma linha existente com valores numéricos idênticos é idempotente. Valores conflitantes são rejeitados. Um lote é atômico: uma linha inválida ou conflitante faz com que nada dessa solicitação seja salvo.

Os dados aparecem imediatamente como **Em medição**. Eles só se tornam representativos após a conclusão. O resultado representativo de um commit é sua execução concluída registrada mais recentemente, incluindo execuções com regressões. Uma execução mais antiga que termina tarde não substitui uma execução concluída registrada mais recentemente. Repetir a conclusão não altera o horário de conclusão nem a ordenação.

Após seis horas sem novos dados de medição salvos, uma execução não concluída passa a **Interrompida**. Retransmissões idênticas não reiniciam esse temporizador. Execuções concluídas e interrompidas rejeitam novos envios; execuções interrompidas também rejeitam a conclusão. Inicie uma nova execução. Os dados permanecem até que você os exclua.

As comparações automáticas de PR fixam o commit merge-base e sua execução concluída representativa no primeiro envio. Uma linha de base ausente permanece indisponível mesmo que seja enviada posteriormente. As comparações automáticas de push percorrem a ancestralidade pelo primeiro pai separadamente para cada linha e usam o ancestral medido mais próximo cuja execução tenha sido concluída até o momento de corte do primeiro envio. Jobs posteriores usam esse mesmo momento de corte. A tabela identifica cada commit e execução de origem efetivamente usados. Comparações com SHA explícito de linha de base não recorrem a ancestrais por linha.

A comparação de dois commits no site dá preferência à execução concluída de branch registrada mais recentemente para cada commit selecionado. Se um commit não tiver uma execução concluída de branch, ela usa a execução concluída de pull request registrada mais recentemente para esse commit. A execução de destino selecionada fornece sua configuração de conjunto de dados armazenada. Novas linhas/colunas e dados ausentes ou incompatíveis não são comparáveis e não causam falha por regressão. Dados excluídos da linha de base não são reconstruídos. Antes da conclusão, uma linha ausente é considerada não coletada; após a conclusão, ela é considerada faltante.

Os gráficos usam commits igualmente espaçados na ordem do primeiro pai. Eles conectam pontos efetivamente medidos através de commits ignorados e nunca propagam nem inventam valores. Filtre por conjunto de dados, branch, coluna, itens, datas e intervalo de commits. Use a tabela de dados para valores exatos e links de execução acessíveis pelo teclado.

## Exclusão e solução de problemas

Gerentes podem excluir permanentemente as medições de um repositório, um conjunto de dados ou uma execução após confirmação. Uma comparação cuja linha de base foi excluída fica indisponível. Quando o Delcore finaliza a exclusão do espaço, o Git Tracker exclui as conexões, os tokens, a configuração e o histórico do espaço.

Em caso de erro de permissão, verifique o espaço selecionado e sua função atual no Git Tracker. Em caso de conflitos de envio, confirme a identidade da execução, a configuração do conjunto de dados e os valores completos das linhas. Para execuções interrompidas ou concluídas, use um novo ID de execução. Em caso de erros de rede/servidor, tente novamente e guarde o ID de correlação da resposta ao pedir ajuda. Nunca compartilhe tokens, credenciais ou medições privadas em relatos públicos.

As medições de execuções de PR continuam disponíveis para inspeção, mas não fornecem pontos para os gráficos da branch de origem nem linhas de base automáticas. Isso impede que medições de forks substituam dados ou configurações da branch de linha de base para um SHA de commit compartilhado.
