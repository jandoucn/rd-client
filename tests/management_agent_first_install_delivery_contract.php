<?php
declare(strict_types=1);

// 离线 API fixture；只接受已有正式 manifest，不生成签名，不访问网络或生产数据库。
// php 本文件 <API仓库> <正式manifest.json> <target_key> <输出目录(必须不存在)>
function fail(int $status, string $message): never { throw new RuntimeException("HTTP $status: $message", $status); }
function db_query(PDO $db, string $sql, array $params = []): PDOStatement {
    $query = $db->prepare($sql); $query->execute($params); return $query;
}
function db_one(PDO $db, string $sql, array $params = []): ?array {
    $row = db_query($db, $sql, $params)->fetch(PDO::FETCH_ASSOC); return $row === false ? null : $row;
}
function db_all(PDO $db, string $sql, array $params = []): array { return db_query($db, $sql, $params)->fetchAll(PDO::FETCH_ASSOC); }
function db_exec(PDO $db, string $sql, array $params = []): void { db_query($db, $sql, $params); }
function check(bool $condition, string $message): void { if (!$condition) throw new RuntimeException($message); }

if ($argc !== 5) throw new InvalidArgumentException('需要 API仓库、正式manifest、MSI target_key、全新输出目录');
[$script, $api, $manifestPath, $key, $output] = $argv;
check(in_array($key, ['windows-x86_64-msi-standard', 'windows-x86_64-msi-sos'], true), '仅支持固定 MSI targets');
check(!file_exists($output) && is_dir(dirname($output)), '输出目录必须不存在且父目录已存在');
require $api.'/sqlite/agent_status.php';
require $api.'/sqlite/agent_operations.php';
$originalBytes = file_get_contents($manifestPath);
$manifest = json_decode($originalBytes, true, 64, JSON_THROW_ON_ERROR);
$provenance = ['input_sha256'=>hash('sha256',$originalBytes),'input_bytes'=>strlen($originalBytes),'mapping'=>'identity'];
if (isset($manifest['assets'])) {
    // 与正式发布器 build_manifest 同构；复用 catalog 原签名，绝不生成 stable.json 原字节。
    check(($manifest['schema'] ?? null) === 2 && ($manifest['tag'] ?? null) === 'v1.5.3-build-2026.10.15-01'
        && ($manifest['build_seq'] ?? null) === 2026101501 && ($manifest['download_base'] ?? null) === 'https://download.yan.life/',
        'INPUT BLOCKED: catalog发布身份不匹配');
    $catalog = $manifest;
    $targets = [];
    foreach ($catalog['assets'] as $asset) {
        $targetKey = $asset['target_key'];
        check(!isset($targets[$targetKey]), 'INPUT BLOCKED: catalog重复target');
        $release = $asset['release'];
        check($release['build_seq'] === 2026101501 && $release['source_commit'] === '9d8d430c01811965c7585d3b57530481eaa15174'
            && $release['source_tag'] === $catalog['tag'], 'INPUT BLOCKED: catalog资产发布身份不匹配');
        check($asset['key'] === 'rustdesk/stable/'.$catalog['tag'].'/'.$asset['name']
            && basename($asset['name']) === $asset['name'], 'INPUT BLOCKED: catalog资产路径不匹配');
        $targets[$targetKey] = $release + ['primary'=>$catalog['download_base'].$asset['key'],
            'mirrors'=>['https://github.com/jandoucn/rustdesk/releases/download/'.$catalog['tag'].'/'.$asset['name']],
            'size'=>$asset['size'],'sha256'=>$asset['sha256'],'signature'=>$asset['signature'],'signature_key_id'=>'yan-release-2026'];
    }
    check(isset($targets[$key]), 'INPUT BLOCKED: catalog缺少请求target');
    $release = $targets[$key];
    $manifest = ['schema'=>2,'catalog_revision'=>$catalog['build_seq'],'version'=>$release['version'],
        'build_number'=>$release['build_number'],'build_seq'=>$release['build_seq'],'product'=>'rustdesk-yan',
        'edition'=>'multi','channel'=>'stable','source_commit'=>$release['source_commit'],'targets'=>$targets];
    $provenance += ['source_url'=>'https://download.yan.life/rustdesk/stable/'.$catalog['tag'].'/catalog.json',
        'mapping_source'=>'.github/scripts/publish-release-to-oss.py:283 build_manifest',
        'catalog_source_commit'=>$release['source_commit']];
    $provenance['mapping'] = 'publisher-build_manifest-equivalent; original target signatures retained';
}
$target = $manifest['targets'][$key] ?? [];
check(($target['build_seq'] ?? $manifest['build_seq'] ?? null) === 2026101501, '必须是冻结的 2026101501 target，不能用当前 latest 替代');
check(($manifest['product'] ?? null) === 'rustdesk-yan' && ($manifest['edition'] ?? null) === 'multi'
    && ($manifest['channel'] ?? null) === 'stable', 'INPUT BLOCKED: 输入不是正式签名更新 manifest，release-snapshot.json 不可替代');
$signature = base64_decode((string)($target['signature'] ?? ''), true);
check(($target['signature_key_id'] ?? null) === 'yan-release-2026' && $signature !== false && strlen($signature) === 64,
    'INPUT BLOCKED: 缺少正式 target 签名');
$release = ($manifest['schema'] ?? 1) >= 2 ? $target : $manifest;
$payload = "rustdesk-update-v1\n";
foreach (['product'=>$manifest['product'], 'edition'=>$manifest['edition'], 'channel'=>$manifest['channel'],
    'version'=>$release['version'] ?? '', 'build_seq'=>$release['build_seq'] ?? 0,
    'source_commit'=>$release['source_commit'] ?? '', 'target_key'=>$key,
    'size'=>$target['size'] ?? 0, 'sha256'=>strtolower((string)($target['sha256'] ?? ''))] as $field=>$value) {
    check(!str_contains((string)$value, "\n") && !str_contains((string)$value, "\r"), 'INPUT BLOCKED: 签名字段包含换行');
    $payload .= $field.'='.$value."\n";
}
check(sodium_crypto_sign_verify_detached($signature, $payload, base64_decode('YNphn2SGjnetwp0bb/uEGpzfQi8OavMWTHCmqPbMuxg=', true)),
    'INPUT BLOCKED: 正式公钥拒绝此 manifest');
$reference = agent_package_reference($manifest, $key);
$db = new PDO('sqlite::memory:');
$db->setAttribute(PDO::ATTR_ERRMODE, PDO::ERRMODE_EXCEPTION);
$db->setAttribute(PDO::ATTR_DEFAULT_FETCH_MODE, PDO::FETCH_ASSOC);
agent_operations_schema($db);
agent_status_schema($db);
$db->exec('CREATE TABLE device_update_keys(device_id TEXT,uuid TEXT,public_key TEXT)');
$db->exec('CREATE TABLE device_deployments(id TEXT,uuid TEXT,pk TEXT)');
$db->exec('CREATE TABLE device_update_policies(id TEXT,uuid TEXT,management_agent_enabled INTEGER)');
$db->exec('CREATE TABLE update_releases(manifest TEXT,active INTEGER,channel TEXT,build_seq INTEGER,published_at INTEGER)');
$id = 't01-offline'; $uuid = 't01-fixture'; $installation = str_repeat('c', 32); $command = str_repeat('a', 32); $now = 1800000000;
$deviceKey = str_repeat("\x01", 32); // 只作为隔离 SQLite 绑定身份，不签发布包或设备 HTTP 请求。
db_exec($db, 'INSERT INTO device_update_keys VALUES(:id,:uuid,:key)', ['id'=>$id,'uuid'=>$uuid,'key'=>base64_encode($deviceKey)]);
db_exec($db, 'INSERT INTO device_update_policies VALUES(:id,:uuid,1)', ['id'=>$id,'uuid'=>$uuid]);
$binding = agent_binding_authorize($db, ['id'=>1,'address_book_scope'=>'all'], $id, $uuid, $installation, true, $now);
$status = ['schema'=>1,'client_id'=>$id,'client_uuid'=>$uuid,'agent_installation_id'=>$installation,
    'boot_id'=>str_repeat('b',32),'session_generation'=>1,'sequence'=>1,'agent'=>['version'=>'1.5.3','build_seq'=>2026101501],
    'service'=>['state'=>'running'],'application'=>['state'=>'ready'],'recovery'=>['effective_enabled'=>true],
    'capabilities'=>['package.reinstall@1'],'package_identity'=>['arch'=>'x86_64','kind'=>'msi','edition'=>str_ends_with($key,'-sos')?'sos':'standard']];
agent_status_store($db, $status, $now);
$args = ['package_id'=>$reference,'target_key'=>$key,'target_build_seq'=>2026101501,'allow_downgrade'=>false];
$envelope = agent_ops_json(['args'=>$args]);
db_exec($db, 'INSERT INTO update_releases VALUES(:manifest,1,\'stable\',2026101501,:now)', ['manifest'=>agent_ops_json($manifest),'now'=>$now]);
db_exec($db, 'INSERT INTO device_agent_commands(command_id,device_id,uuid,installation_id,generation,type,type_version,envelope,digest,status,idempotency_key,request_digest,created_at,expires_at,updated_at,actor_id) VALUES(:command,:id,:uuid,:installation,:generation,\'package.reinstall\',1,:envelope,:digest,\'accepted\',\'t01\',:request_digest,:now,:expires,:updated,1)',
    ['command'=>$command,'id'=>$id,'uuid'=>$uuid,'installation'=>$installation,'generation'=>$binding['generation'],
     'envelope'=>$envelope,'digest'=>hash('sha256',$envelope),'request_digest'=>hash('sha256',$envelope),'now'=>$now,'expires'=>$now+900,'updated'=>$now]);
$reply = agent_package_resolve($db, $id, $uuid, $installation, $binding['generation'], $command, $now);
check($reply['package_id'] === $reference && $reply['rollback']['package_id'] === $reference, '同版本重装必须解析目标及当前批准回滚包');
check(agent_ops_json($reply['manifest']) === agent_ops_json($manifest), 'API 必须保留正式签名 manifest');
check(db_one($db, 'SELECT status FROM device_agent_commands WHERE command_id=:id', ['id'=>$command])['status'] === 'accepted', 'resolve 不得伪造 committed');
$before = agent_ops_json(db_all($db, 'SELECT * FROM device_agent_status'));
$commandBefore = agent_ops_json(db_all($db, 'SELECT * FROM device_agent_commands'));
db_exec($db, 'UPDATE update_releases SET active=0');
try {
    agent_package_resolve($db, $id, $uuid, $installation, $binding['generation'], $command, $now);
    throw new RuntimeException('撤销后错误地返回了批准包');
} catch (RuntimeException $error) {
    check($error->getCode() === 409, '撤销必须返回 409，实际：'.$error->getMessage());
}
check(agent_ops_json(db_all($db, 'SELECT * FROM device_agent_status')) === $before, '拒绝必须保留旧 status');
check(agent_ops_json(db_all($db, 'SELECT * FROM device_agent_commands')) === $commandBefore, '撤销拒绝必须保留旧 command');
check((int)db_one($db, 'SELECT active FROM update_releases')['active'] === 0, '撤销必须真实持久化');
check(mkdir($output), '无法建立输出目录');
check(file_put_contents($output.'/resolved.json', json_encode($reply, JSON_PRETTY_PRINT|JSON_UNESCAPED_SLASHES|JSON_THROW_ON_ERROR)) !== false, '无法写 resolved.json');
check(file_put_contents($output.'/original-input.json', $originalBytes) === strlen($originalBytes), '无法保留原始catalog字节');
$receipt = ['result'=>'pass','target_key'=>$key,'package_id'=>$reference,'signature_verified'=>true,
    'positive_resolve'=>true,'revoked_error'=>409,'status_preserved'=>true,'command_preserved'=>true,
    'release_active_after'=>0,'provenance'=>$provenance,
    'sqlite_status_rows'=>db_all($db,'SELECT * FROM device_agent_status'),
    'sqlite_command_rows'=>db_all($db,'SELECT * FROM device_agent_commands')];
check(file_put_contents($output.'/fixture-receipt.json', json_encode($receipt,JSON_PRETTY_PRINT|JSON_UNESCAPED_SLASHES|JSON_THROW_ON_ERROR)) !== false, '无法写fixture回执');
echo "API SQLite fixture PASS；正式公钥验签通过；Rust package_gate::verify 将独立复验；未生成 first-install.json\n";
