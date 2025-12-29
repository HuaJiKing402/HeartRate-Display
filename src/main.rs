use anyhow::Result;
use btleplug::api::{ bleuuid::uuid_from_u16, Central, CharPropFlags, Manager as _, Peripheral as _, ScanFilter, };
use btleplug::platform::{Adapter, Manager, Peripheral};
use futures::stream::StreamExt;
use std::time::Duration;
use uuid::Uuid;

// 蓝牙心率服务标准UUID (0x180D)
const HEART_RATE_SERVICE_UUID: Uuid = uuid_from_u16(0x180D);
// 心率测量特征值UUID (0x2A37)
const HEART_RATE_MEASUREMENT_UUID: Uuid = uuid_from_u16(0x2A37);

/// 解析心率测量数据（符合蓝牙心率规范 GATT 0x2A37）
fn parse_heart_rate_data(data: &[u8]) -> Option<u16> {
    if data.is_empty() {
        return None;
    }

    // 第一个字节是Flags，定义心率数据格式
    let flags = data[0];
    // Bit0: 0=心率为uint8，1=uint16
    let is_16bit_hr = (flags & 0x01) != 0;

    let heart_rate = if is_16bit_hr {
        // uint16（小端序）
        if data.len() < 3 {
            return None;
        }
        u16::from_le_bytes([data[1], data[2]])
    } else {
        // uint8
        if data.len() < 2 {
            return None;
        }
        data[1] as u16
    };

    Some(heart_rate)
}

/// 查找支持心率服务的BLE设备
async fn find_heart_rate_device(adapter: &Adapter) -> Option<Peripheral> {
    // 扫描2秒获取周边设备
    adapter.start_scan(ScanFilter::default()).await.ok()?;
    tokio::time::sleep(Duration::from_secs(2)).await;

    // 遍历所有扫描到的设备
    let peripherals = adapter.peripherals().await.ok()?;
    for peripheral in peripherals {
        let properties = peripheral.properties().await.ok()??;

        // 筛选包含心率服务的设备
        if properties.services.contains(&HEART_RATE_SERVICE_UUID) {
            let device_name = properties.local_name.unwrap_or_else(|| "未知设备".to_string());
            println!("\n找到心率设备: {}", device_name);
            println!("设备地址: {:?}", peripheral.address());
            return Some(peripheral);
        }
    }

    None
}

#[tokio::main]
async fn main() -> Result<()> {
    // 初始化日志（可选，方便调试）
    pretty_env_logger::init();

    // 1. 初始化BLE管理器
    let manager = Manager::new().await?;

    // 2. 获取第一个可用的蓝牙适配器
    let adapters = manager.adapters().await?;
    let adapter = adapters.into_iter().next().ok_or_else(|| {
        anyhow::anyhow!("未找到蓝牙适配器，请确保蓝牙已开启")
    })?;
    println!("使用蓝牙适配器: {:?}", adapter.adapter_info().await?);

    // 3. 查找心率设备
    println!("正在扫描心率设备...");
    let heart_rate_device = find_heart_rate_device(&adapter).await.ok_or_else(|| {
        anyhow::anyhow!("未找到心率设备，请确保设备已开启并处于可连接状态")
    })?;

    // 4. 连接设备
    if !heart_rate_device.is_connected().await? {
        println!("正在连接设备...");
        heart_rate_device.connect().await?;
        println!("设备连接成功！");
    }

    // 5. 发现设备的服务和特征
    heart_rate_device.discover_services().await?;
    let characteristics = heart_rate_device.characteristics();

    // 6. 找到心率测量特征并订阅通知
    let hr_characteristic = characteristics
        .iter()
        .find(|c| {
            c.uuid == HEART_RATE_MEASUREMENT_UUID
                && c.properties.contains(CharPropFlags::NOTIFY)
        })
        .ok_or_else(|| {
            anyhow::anyhow!("设备不支持心率测量通知特征")
        })?;

    // 订阅心率通知
    heart_rate_device.subscribe(hr_characteristic).await?;
    println!("\n已订阅心率通知，开始接收数据（按Ctrl+C退出）...\n");

    // 7. 监听心率通知流并解析数据
    let mut notification_stream = heart_rate_device.notifications().await?;
    while let Some(notification) = notification_stream.next().await {
        if let Some(heart_rate) = parse_heart_rate_data(&notification.value) {
            println!("当前心率: {} BPM", heart_rate);
        } else {
            println!("解析心率数据失败: {:?}", notification.value);
        }
    }

    // 8. 断开连接（实际场景中可根据需求处理）
    heart_rate_device.disconnect().await?;
    println!("设备已断开连接");

    Ok(())
}