// 證據畫布 template：先放結論指標，再放一張圖。規則與 kunsu 模組說明見 ../../canvas.md。
import { LineChart, Line, XAxis, YAxis, CartesianGrid, Tooltip } from 'recharts'
import { Grid, Stat, Panel, ChartFrame, chart, series } from 'kunsu'

const data = [
  { day: '10/06', value: 2.1 },
  { day: '10/07', value: 1.9 },
  { day: '10/08', value: 1.6 },
]

export default function Canvas() {
  return (
    <div className="flex flex-col gap-3">
      <Grid>
        <Stat label="ROAS" value="1.6" delta={-0.24} spark={data.map((d) => d.value)} note="前 7 天平均 2.1" />
      </Grid>
      <Panel title="ROAS 走勢" note="Meta API">
        <ChartFrame height={200}>
          <LineChart data={data} margin={chart.margin}>
            <CartesianGrid {...chart.grid} />
            <XAxis dataKey="day" {...chart.axis} />
            <YAxis {...chart.yAxis} />
            <Tooltip {...chart.tooltip} />
            <Line dataKey="value" name="ROAS" stroke={series[0]} {...chart.line} />
          </LineChart>
        </ChartFrame>
      </Panel>
    </div>
  )
}
